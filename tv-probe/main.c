/* Vita TV Probe: original diagnostic code, MIT license (see LICENSE).
 * The local control clip is unencrypted. There is no service DRM implementation.
 */
#include <psp2/apputil.h>
#include <psp2/audioout.h>
#include <psp2/avplayer.h>
#include <psp2/ctrl.h>
#include <psp2/gxm.h>
#include <psp2/io/fcntl.h>
#include <psp2/io/stat.h>
#include <psp2/kernel/modulemgr.h>
#include <psp2/kernel/processmgr.h>
#include <psp2/kernel/sysmem.h>
#include <psp2/kernel/threadmgr.h>
#include <psp2/sysmodule.h>
#include <vita2d.h>
#include <malloc.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define DATA_DIR "ux0:data/greenvita-tv-probe"
#define LOG_PATH DATA_DIR "/probe.log"
#define WHITE RGBA8(236, 242, 250, 255)
#define MUTED RGBA8(171, 184, 205, 255)
#define ACCENT RGBA8(59, 220, 173, 255)
#define WARN RGBA8(255, 198, 100, 255)

typedef struct { const char *name; const char *url; } Service;
static const Service services[] = {
    {"Apple TV", "https://tv.apple.com/"},
    {"Netflix", "https://www.netflix.com/login"},
    {"Hulu", "https://www.hulu.com/"},
    {"HBO Max", "https://www.hbomax.com/"}
};
static const char *observations[] = {
    "Not tested", "Page loaded only", "Signed in only", "Playback error",
    "Moving video WITHOUT sound", "Protected title with video AND sound"
};
static const char *observation_codes[] = {
    "NOT_TESTED", "PAGE_ONLY", "SIGNIN_ONLY", "PLAYBACK_ERROR",
    "VIDEO_NO_AUDIO", "USER_REPORTED_PROTECTED_VIDEO_AND_AUDIO"
};
static int recorded[4];
static SceUID logfile = -1;
static SceUID mutex = -1, audio_thread = -1;
/* An opaque pointer-sized handle, not a signed SceUID/status code. Real Vita
 * handles commonly have bit 31 set. Only zero means no player. */
static SceAvPlayerHandle player = 0;
static int audio_running, paused, volume = 40;
static unsigned video_frames, audio_blocks;
static int audio_error, audio_port = -1, last_audio_volume = -1;
static uint64_t started_at, last_stats, media_time;
static int have_frame;
static vita2d_texture frame_texture;
static SceAvPlayerFrameInfo frame;
static vita2d_pgf *font;
static char status[160] = "Choose a test. Service playback is NOT verified.";
static char firmware[32] = "unknown";
static int browser_ready, av_ready;

static void log_line(const char *fmt, ...) {
    if (logfile < 0) return;
    char body[512], line[600];
    va_list args;
    va_start(args, fmt);
    vsnprintf(body, sizeof(body), fmt, args);
    va_end(args);
    int n = snprintf(line, sizeof(line), "%llu ms %s\n",
        (unsigned long long)(sceKernelGetProcessTimeWide() / 1000), body);
    if (n > 0 && n < (int)sizeof(line)) sceIoWrite(logfile, line, n);
}

static void label(float x, float y, unsigned color, const char *fmt, ...) {
    char text[256];
    va_list args;
    va_start(args, fmt);
    vsnprintf(text, sizeof(text), fmt, args);
    va_end(args);
    vita2d_pgf_draw_text(font, x, y, color, 1.0f, text);
}

static void *allocate(void *ctx, uint32_t alignment, uint32_t size) {
    (void)ctx;
    if (alignment < sizeof(void *)) alignment = sizeof(void *);
    return memalign(alignment, size);
}

static void deallocate(void *ctx, void *ptr) { (void)ctx; free(ptr); }

static void *allocate_frame(void *ctx, uint32_t alignment, uint32_t size) {
    (void)ctx;
    if (alignment < 0x40000) alignment = 0x40000;
    if (!size || (alignment & (alignment - 1)) || size > UINT32_MAX - alignment) return NULL;
    uint32_t rounded = (size + alignment - 1) & ~(alignment - 1);
    SceKernelAllocMemBlockOpt opt;
    memset(&opt, 0, sizeof(opt));
    opt.size = sizeof(opt);
    opt.attr = 4; /* SCE_KERNEL_ALLOC_MEMBLOCK_ATTR_HAS_ALIGNMENT */
    opt.alignment = alignment;
    SceUID block = sceKernelAllocMemBlock("tv-probe-frame",
        SCE_KERNEL_MEMBLOCK_TYPE_USER_CDRAM_RW, rounded, &opt);
    if (block < 0) return NULL;
    void *ptr = NULL;
    if (sceKernelGetMemBlockBase(block, &ptr) < 0 ||
        sceGxmMapMemory(ptr, rounded, SCE_GXM_MEMORY_ATTRIB_READ) < 0) {
        sceKernelFreeMemBlock(block);
        return NULL;
    }
    return ptr;
}

static void deallocate_frame(void *ctx, void *ptr) {
    (void)ctx;
    if (!ptr) return;
    SceUID block = sceKernelFindMemBlockByAddr(ptr, 0);
    sceGxmUnmapMemory(ptr);
    if (block >= 0) sceKernelFreeMemBlock(block);
}

/* The mutex covers AVPlayer access and synchronous audio output. Shutdown joins
 * this worker before releasing the player or any player-owned frame memory. */
static int audio_worker(SceSize arglen, void *argp) {
    (void)arglen; (void)argp;
    unsigned last_size = 0, last_rate = 0, last_channels = 0;
    for (;;) {
        sceKernelLockMutex(mutex, 1, NULL);
        if (!audio_running) { sceKernelUnlockMutex(mutex, 1); break; }
        SceAvPlayerFrameInfo sound;
        memset(&sound, 0, sizeof(sound));
        if (!paused && sceAvPlayerGetAudioData(player, &sound)) {
            unsigned channels = sound.details.audio.channelCount;
            unsigned rate = sound.details.audio.sampleRate;
            unsigned bytes = sound.details.audio.size;
            int frames = channels ? (int)(bytes / (channels * sizeof(int16_t))) : 0;
            if ((channels != 1 && channels != 2) || !rate || !frames || !sound.pData) {
                audio_error = -1;
            } else {
                int mode = channels == 2 ? SCE_AUDIO_OUT_MODE_STEREO : SCE_AUDIO_OUT_MODE_MONO;
                if (audio_port < 0) {
                    audio_port = sceAudioOutOpenPort(SCE_AUDIO_OUT_PORT_TYPE_MAIN, frames, rate, mode);
                    if (audio_port < 0) audio_error = audio_port;
                } else if (bytes != last_size || rate != last_rate || channels != last_channels) {
                    int result = sceAudioOutSetConfig(audio_port, frames, rate, mode);
                    if (result < 0) audio_error = result;
                }
                last_size = bytes; last_rate = rate; last_channels = channels;
                if (audio_port >= 0 && !audio_error) {
                    if (last_audio_volume != volume) {
                        int levels[2] = {SCE_AUDIO_VOLUME_0DB * volume / 100, SCE_AUDIO_VOLUME_0DB * volume / 100};
                        int result = sceAudioOutSetVolume(audio_port,
                            SCE_AUDIO_VOLUME_FLAG_L_CH | SCE_AUDIO_VOLUME_FLAG_R_CH, levels);
                        if (result < 0) audio_error = result;
                        last_audio_volume = volume;
                    }
                    int result = sceAudioOutOutput(audio_port, sound.pData);
                    if (result < 0) audio_error = result;
                    else audio_blocks++;
                }
            }
        }
        sceKernelUnlockMutex(mutex, 1);
        sceKernelDelayThread(1000);
    }
    if (audio_port >= 0) {
        sceAudioOutOutput(audio_port, NULL);
        sceAudioOutReleasePort(audio_port);
        audio_port = -1;
    }
    return 0;
}

static void log_stats(const char *reason) {
    unsigned blocks; int err;
    sceKernelLockMutex(mutex, 1, NULL);
    blocks = audio_blocks; err = audio_error;
    sceKernelUnlockMutex(mutex, 1);
    double elapsed = (sceKernelGetProcessTimeWide() - started_at) / 1000000.0;
    log_line("CONTROL %s wall_seconds=%.2f frames=%u audio_blocks=%u audio_error=0x%08x media_ms=%llu paused=%d volume=%d",
        reason, elapsed, video_frames, blocks, (unsigned)err,
        (unsigned long long)media_time, paused, volume);
}

static void stop_player(void) {
    if (player == 0) return;
    vita2d_wait_rendering_done();
    sceKernelLockMutex(mutex, 1, NULL);
    audio_running = 0;
    sceKernelUnlockMutex(mutex, 1);
    if (audio_thread >= 0) {
        sceKernelWaitThreadEnd(audio_thread, NULL, NULL);
        sceKernelDeleteThread(audio_thread);
        audio_thread = -1;
    }
    log_stats("stop");
    int stop_result = sceAvPlayerStop(player);
    int close_result = sceAvPlayerClose(player);
    log_line("CONTROL stop_result=0x%08x close_result=0x%08x", (unsigned)stop_result, (unsigned)close_result);
    player = 0; have_frame = 0; paused = 0;
}

static void start_player(void) {
    stop_player();
    if (!av_ready) { snprintf(status, sizeof(status), "AVPlayer module unavailable; see probe.log."); return; }
    SceAvPlayerInitData init;
    memset(&init, 0, sizeof(init));
    init.memoryReplacement.allocate = allocate;
    init.memoryReplacement.deallocate = deallocate;
    init.memoryReplacement.allocateTexture = allocate_frame;
    init.memoryReplacement.deallocateTexture = deallocate_frame;
    init.basePriority = 0xA0;
    init.numOutputVideoFrameBuffers = 3;
    init.autoStart = SCE_TRUE;
    init.defaultLanguage = "eng";
    player = sceAvPlayerInit(&init);
    log_line("CONTROL init_handle=0x%08x present=%d", (unsigned)player, player != 0);
    if (player == 0) { snprintf(status, sizeof(status), "Player initialization returned a null handle; see probe.log."); return; }
    started_at = last_stats = sceKernelGetProcessTimeWide();
    video_frames = audio_blocks = 0; audio_error = 0; media_time = 0;
    last_audio_volume = -1; paused = 0; have_frame = 0;
    int result = sceAvPlayerSetLooping(player, SCE_TRUE);
    log_line("CONTROL loop_result=0x%08x", (unsigned)result);
    result = sceAvPlayerAddSource(player, "app0:control.mp4");
    log_line("CONTROL add_source=0x%08x", (unsigned)result);
    if (result < 0) {
        snprintf(status, sizeof(status), "Clip open failed: 0x%08x", (unsigned)result);
        stop_player(); return;
    }
    audio_thread = sceKernelCreateThread("tv-probe-audio", audio_worker, 0x10000100,
        0x10000, 0, 0, NULL);
    log_line("CONTROL audio_thread_create=0x%08x", (unsigned)audio_thread);
    if (audio_thread < 0) {
        snprintf(status, sizeof(status), "Audio thread failed: 0x%08x", (unsigned)audio_thread);
        stop_player(); return;
    }
    audio_running = 1;
    result = sceKernelStartThread(audio_thread, 0, NULL);
    log_line("CONTROL audio_thread_start=0x%08x", (unsigned)result);
    if (result < 0) {
        sceKernelDeleteThread(audio_thread); audio_thread = -1; audio_running = 0;
        snprintf(status, sizeof(status), "Audio start failed: 0x%08x", (unsigned)result);
        stop_player(); return;
    }
    snprintf(status, sizeof(status), "Local unencrypted clip. Confirm moving pattern AND audible tone yourself.");
}

static void control_input(unsigned pressed) {
    if (pressed & SCE_CTRL_CROSS) {
        if (player == 0) start_player();
        else {
            sceKernelLockMutex(mutex, 1, NULL);
            int result = paused ? sceAvPlayerResume(player) : sceAvPlayerPause(player);
            if (result >= 0) paused = !paused;
            sceKernelUnlockMutex(mutex, 1);
            log_line("CONTROL pause_toggle result=0x%08x paused=%d", (unsigned)result, paused);
        }
    }
    if (player != 0 && (pressed & (SCE_CTRL_LEFT | SCE_CTRL_RIGHT))) {
        vita2d_wait_rendering_done();
        sceKernelLockMutex(mutex, 1, NULL);
        int64_t target = (int64_t)sceAvPlayerCurrentTime(player) + ((pressed & SCE_CTRL_RIGHT) ? 3000 : -3000);
        if (target < 0) target = 0;
        if (target > 11000) target = 11000;
        have_frame = 0;
        int result = sceAvPlayerJumpToTime(player, (uint64_t)target);
        sceKernelUnlockMutex(mutex, 1);
        log_line("CONTROL seek target_ms=%lld result=0x%08x", (long long)target, (unsigned)result);
    }
    if (pressed & (SCE_CTRL_LTRIGGER | SCE_CTRL_RTRIGGER)) {
        sceKernelLockMutex(mutex, 1, NULL);
        volume += (pressed & SCE_CTRL_RTRIGGER) ? 10 : -10;
        if (volume < 0) volume = 0;
        if (volume > 100) volume = 100;
        sceKernelUnlockMutex(mutex, 1);
        log_line("CONTROL volume_requested=%d", volume);
    }
    if (pressed & SCE_CTRL_SQUARE) { stop_player(); start_player(); }
}

static void update_video(void) {
    if (player == 0) return;
    sceKernelLockMutex(mutex, 1, NULL);
    if (!paused && sceAvPlayerGetVideoData(player, &frame)) {
        memset(&frame_texture, 0, sizeof(frame_texture));
        int result = sceGxmTextureInitLinear(&frame_texture.gxm_tex, frame.pData,
            SCE_GXM_TEXTURE_FORMAT_YVU420P2_CSC1, frame.details.video.width,
            frame.details.video.height, 0);
        have_frame = result >= 0;
        if (have_frame) {
            video_frames++;
            vita2d_texture_set_filters(&frame_texture, SCE_GXM_TEXTURE_FILTER_LINEAR, SCE_GXM_TEXTURE_FILTER_LINEAR);
            if (video_frames == 1) log_line("CONTROL first_frame width=%u height=%u", frame.details.video.width, frame.details.video.height);
        } else log_line("CONTROL texture_error=0x%08x", (unsigned)result);
    }
    media_time = sceAvPlayerCurrentTime(player);
    sceKernelUnlockMutex(mutex, 1);
    uint64_t now = sceKernelGetProcessTimeWide();
    if (now - last_stats >= 5000000) { log_stats("sample"); last_stats = now; }
}

static void launch_service(int index) {
    if (!browser_ready) { snprintf(status, sizeof(status), "Browser handoff unavailable; see probe.log."); return; }
    SceAppUtilWebBrowserParam web;
    memset(&web, 0, sizeof(web));
    web.str = services[index].url;
    web.strlen = strlen(web.str);
    int result = sceAppUtilLaunchWebBrowser(&web);
    log_line("SERVICE name=%s browser_launch=0x%08x protected_playback=NOT_VERIFIED", services[index].name, (unsigned)result);
    if (result < 0) snprintf(status, sizeof(status), "Browser launch failed: 0x%08x. This is not a playback result.", (unsigned)result);
    else snprintf(status, sizeof(status), "Browser opened. Return here using PS; record only what you observed.");
}

int main(void) {
    sceIoMkdir(DATA_DIR, 0777);
    logfile = sceIoOpen(LOG_PATH, SCE_O_WRONLY | SCE_O_CREAT | SCE_O_APPEND, 0666);
    log_line("SESSION app=GVTVPRB01 version=0.3 protected_playback=NOT_VERIFIED");
    SceKernelSystemSwVersion sw;
    memset(&sw, 0, sizeof(sw)); sw.size = sizeof(sw);
    if (sceKernelGetSystemSwVersion(&sw) >= 0) snprintf(firmware, sizeof(firmware), "%s", sw.versionString);
    log_line("DEVICE model_api=0x%08x firmware_api=%s may_be_spoofed=1 exact_model_requires_user", (unsigned)sceKernelGetModel(), firmware);
    FILE *build = fopen("app0:build-info.json", "r");
    if (build) { char line[400]; while (fgets(line, sizeof(line), build)) log_line("BUILD %s", line); fclose(build); }
    mutex = sceKernelCreateMutex("tv-probe-lock", 0, 0, NULL);
    if (mutex < 0) { log_line("FATAL mutex=0x%08x", (unsigned)mutex); if (logfile >= 0) sceIoClose(logfile); return 1; }
    int result = sceSysmoduleLoadModule(SCE_SYSMODULE_AVPLAYER);
    av_ready = result >= 0;
    log_line("AVPLAYER module=0x%08x", (unsigned)result);
    result = sceSysmoduleLoadModule(SCE_SYSMODULE_APPUTIL);
    log_line("APPUTIL module=0x%08x", (unsigned)result);
    if (result >= 0) {
        SceAppUtilInitParam init; SceAppUtilBootParam boot;
        memset(&init, 0, sizeof(init)); memset(&boot, 0, sizeof(boot));
        result = sceAppUtilInit(&init, &boot);
        browser_ready = result >= 0;
        log_line("APPUTIL init=0x%08x", (unsigned)result);
    }
    if (vita2d_init() < 0) { log_line("FATAL vita2d_init"); sceKernelExitProcess(1); return 1; }
    vita2d_set_vblank_wait(1);
    vita2d_set_clear_color(RGBA8(15, 22, 36, 255));
    font = vita2d_load_default_pgf();
    if (!font) { log_line("FATAL font"); vita2d_fini(); sceKernelExitProcess(1); return 1; }
    sceCtrlSetSamplingMode(SCE_CTRL_MODE_DIGITAL);
    int mode = 0, selected = 0, service = 0, observation = 0, running = 1;
    unsigned previous = 0;
    const char *menu[] = {"1. Bundled video + sound control", "2. Apple TV browser test", "3. Netflix browser test", "4. Hulu browser test", "5. HBO Max browser test", "6. Results and log location", "Exit"};
    while (running) {
        vita2d_wait_rendering_done();
        SceCtrlData pad; memset(&pad, 0, sizeof(pad));
        sceCtrlPeekBufferPositive(0, &pad, 1);
        unsigned pressed = pad.buttons & ~previous; previous = pad.buttons;
        if (mode == 0) {
            if (pressed & SCE_CTRL_DOWN) selected = (selected + 1) % 7;
            if (pressed & SCE_CTRL_UP) selected = (selected + 6) % 7;
            if (pressed & SCE_CTRL_CROSS) {
                if (selected == 0) { mode = 1; start_player(); }
                else if (selected <= 4) { mode = 2; service = selected - 1; observation = recorded[service]; }
                else if (selected == 5) mode = 3;
                else running = 0;
            }
        } else if (pressed & SCE_CTRL_CIRCLE) {
            stop_player(); mode = 0;
            snprintf(status, sizeof(status), "Test finished. Logs are local; no service playback is certified.");
        } else if (mode == 1) control_input(pressed);
        else if (mode == 2) {
            if (pressed & SCE_CTRL_CROSS) launch_service(service);
            if (pressed & SCE_CTRL_RIGHT) observation = (observation + 1) % 6;
            if (pressed & SCE_CTRL_LEFT) observation = (observation + 5) % 6;
            if (pressed & SCE_CTRL_SQUARE) {
                recorded[service] = observation;
                log_line("SERVICE name=%s manual_observation=%s source=USER verified_by_build=0",
                    services[service].name, observation_codes[observation]);
                snprintf(status, sizeof(status), "User observation recorded. Share title, duration and errors separately.");
            }
        }
        if (mode == 1) update_video();
        vita2d_start_drawing(); vita2d_clear_screen();
        label(28, 35, ACCENT, "VITA TV PROBE 0.3   /   diagnostic build");
        if (mode == 0) {
            label(28, 75, WARN, "No protected streaming service has passed on this device.");
            for (int i = 0; i < 7; i++) {
                if (i == selected) vita2d_draw_rectangle(22, 95 + i * 43, 916, 38, RGBA8(34, 56, 74, 255));
                label(38, 122 + i * 43, i == selected ? ACCENT : WHITE, "%s", menu[i]);
            }
            label(28, 440, MUTED, "Up/Down: choose   X: open   |   Firmware API: %s (may be spoofed)", firmware);
        } else if (mode == 1) {
            if (have_frame && frame.details.video.width && frame.details.video.height)
                vita2d_draw_texture_scale(&frame_texture, 170, 55,
                    620.0f / frame.details.video.width, 351.0f / frame.details.video.height);
            else label(170, 210, WARN, "Waiting for a decoded frame. See log if this persists.");
            unsigned blocks; int err;
            sceKernelLockMutex(mutex, 1, NULL); blocks = audio_blocks; err = audio_error; sceKernelUnlockMutex(mutex, 1);
            label(28, 433, WHITE, "%s  %.1fs  Frames %u  Audio blocks %u  Volume %d%%  Audio error %08x",
                paused ? "Paused" : "Control", media_time / 1000.0, video_frames, blocks, volume, (unsigned)err);
            label(28, 462, MUTED, "X: pause/play   Left/Right: seek 3s   L/R: volume   Square: restart   O: back");
        } else if (mode == 2) {
            label(28, 87, WHITE, "%s - official website", services[service].name);
            label(28, 123, MUTED, "%s", services[service].url);
            label(28, 170, WARN, "System browser test. No embedded player or DRM adapter is supplied.");
            label(28, 208, WHITE, "X: open browser. Sign in there, then try a protected episode or movie.");
            label(28, 244, WHITE, "Return with PS and select what happened. O: back.");
            label(28, 300, ACCENT, "Left/Right: %s", observations[observation]);
            label(28, 340, WHITE, "Square: record this observation. Page/sign-in success is NOT playback.");
            label(28, 391, MUTED, "Check sound, pause, seek and subtitles; note title and session length.");
            label(28, 427, MUTED, "Do not enter credentials anywhere except the service's own HTTPS page.");
        } else if (mode == 3) {
            label(28, 85, WHITE, "This session's manually recorded observations:");
            for (int i = 0; i < 4; i++) label(28, 134 + i * 45, MUTED, "%s: %s", services[i].name, observations[recorded[i]]);
            label(28, 350, ACCENT, "Log: %s", LOG_PATH);
            label(28, 390, WHITE, "Counters show API activity; confirm visible video and audible sound yourself.");
            label(28, 428, MUTED, "No passwords, tokens, cookies or protected media are written to the log.");
            label(28, 463, WHITE, "O: back");
        }
        label(28, 513, logfile < 0 ? WARN : MUTED, "%s", logfile < 0 ? "Log could not be opened. Record results manually." : status);
        vita2d_end_drawing(); vita2d_swap_buffers();
    }
    stop_player();
    log_line("SESSION end");
    if (logfile >= 0) sceIoClose(logfile);
    vita2d_wait_rendering_done(); vita2d_free_pgf(font); vita2d_fini();
    if (browser_ready) sceAppUtilShutdown();
    sceKernelDeleteMutex(mutex);
    sceKernelExitProcess(0);
    return 0;
}
