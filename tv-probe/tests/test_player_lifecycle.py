#!/usr/bin/env python3
"""Compile the actual lifecycle/control functions against host API doubles.

No Vita decoder, threads, GPU, sound or service playback are exercised here.
Source extraction avoids maintaining a second copy of the production logic.
"""
from pathlib import Path
import os
import re
import subprocess
import tempfile

source = (Path(__file__).resolve().parents[1] / "main.c").read_text()


def function(name):
    start = source.index(f"static void {name}(")
    opening = source.index("{", start)
    depth = 1
    end = opening + 1
    while depth:
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    return source[start:end]


# Catch signed validity comparisons and the old -1 sentinel anywhere in the app,
# including paths outside the functions exercised below.
assert not re.search(r"\bplayer\s*(?:<|>|<=|>=)\s*0", source)
assert not re.search(r"\bplayer\s*=\s*-1\b", source)
declaration = re.search(r"^static SceAvPlayerHandle player = .*;", source, re.M).group()

stubs = r'''
#include <assert.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
typedef int32_t SceAvPlayerHandle;
typedef struct {
    struct { void *(*allocate)(void *, uint32_t, uint32_t);
        void (*deallocate)(void *, void *);
        void *(*allocateTexture)(void *, uint32_t, uint32_t);
        void (*deallocateTexture)(void *, void *); } memoryReplacement;
    int basePriority, numOutputVideoFrameBuffers, autoStart;
    const char *defaultLanguage;
} SceAvPlayerInitData;
typedef struct { void *pData; struct { struct { unsigned width, height; } video; } details; } SceAvPlayerFrameInfo;
typedef struct { int gxm_tex; } vita2d_texture;
#define SCE_TRUE 1
#define SCE_CTRL_CROSS 1
#define SCE_CTRL_LEFT 2
#define SCE_CTRL_RIGHT 4
#define SCE_CTRL_LTRIGGER 8
#define SCE_CTRL_RTRIGGER 16
#define SCE_CTRL_SQUARE 32
#define SCE_GXM_TEXTURE_FORMAT_YVU420P2_CSC1 1
#define SCE_GXM_TEXTURE_FILTER_LINEAR 1
static int mutex, audio_thread = -1, audio_running, paused, volume = 40;
static int audio_error, last_audio_volume = -1, have_frame, av_ready = 1;
static unsigned video_frames, audio_blocks;
static uint64_t started_at, last_stats, media_time;
static vita2d_texture frame_texture;
static SceAvPlayerFrameInfo frame;
static char status[160];
static uint32_t next_handle;
static int live, init_calls, add_calls, close_calls, pause_calls, resume_calls;
static int seek_calls, get_video_calls, joined, deleted, stops;
static int fail_add, fail_create, fail_start;
static void assert_live(SceAvPlayerHandle h) { assert(live == 1); assert((uint32_t)h == next_handle); }
static void log_line(const char *fmt, ...) { (void)fmt; }
static void log_stats(const char *reason) { (void)reason; }
static void *allocate(void *c, uint32_t a, uint32_t n) { (void)c; (void)a; (void)n; return NULL; }
static void deallocate(void *c, void *p) { (void)c; (void)p; }
static void *allocate_frame(void *c, uint32_t a, uint32_t n) { return allocate(c, a, n); }
static void deallocate_frame(void *c, void *p) { deallocate(c, p); }
static void vita2d_wait_rendering_done(void) {}
static int sceKernelLockMutex(int m, int n, void *t) { (void)m; (void)n; (void)t; return 0; }
static int sceKernelUnlockMutex(int m, int n) { (void)m; (void)n; return 0; }
static uint64_t sceKernelGetProcessTimeWide(void) { static uint64_t t; return t += 1000; }
static int sceKernelWaitThreadEnd(int t, void *a, void *b) { (void)t; (void)a; (void)b; assert(!audio_running); ++joined; return 0; }
static int sceKernelDeleteThread(int t) { (void)t; ++deleted; return 0; }
static int audio_worker(unsigned n, void *p) { (void)n; (void)p; return 0; }
static int sceKernelCreateThread(const char *n, int (*w)(unsigned, void *), int p, int s, int a, int c, void *o) {
    (void)n; (void)w; (void)p; (void)s; (void)a; (void)c; (void)o;
    return fail_create ? -2 : 9;
}
static int sceKernelStartThread(int t, unsigned n, void *a) { (void)t; (void)n; (void)a; return fail_start ? -3 : 0; }
static SceAvPlayerHandle sceAvPlayerInit(SceAvPlayerInitData *data) {
    (void)data; assert(live == 0); ++init_calls; live = next_handle != 0;
    return (SceAvPlayerHandle)next_handle;
}
static int sceAvPlayerSetLooping(SceAvPlayerHandle h, int loop) { assert_live(h); (void)loop; return 0; }
static int sceAvPlayerAddSource(SceAvPlayerHandle h, const char *path) {
    assert_live(h); assert(!strcmp(path, "app0:control.mp4")); ++add_calls;
    return fail_add ? -4 : 0;
}
static int sceAvPlayerStop(SceAvPlayerHandle h) { assert_live(h); assert(!audio_running); ++stops; return 0; }
static int sceAvPlayerClose(SceAvPlayerHandle h) { assert_live(h); assert(!audio_running); assert(audio_thread < 0); live = 0; ++close_calls; return 0; }
static int sceAvPlayerPause(SceAvPlayerHandle h) { assert_live(h); ++pause_calls; return 0; }
static int sceAvPlayerResume(SceAvPlayerHandle h) { assert_live(h); ++resume_calls; return 0; }
static uint64_t sceAvPlayerCurrentTime(SceAvPlayerHandle h) { assert_live(h); return 4000; }
static int sceAvPlayerJumpToTime(SceAvPlayerHandle h, uint64_t t) { assert_live(h); assert(t <= 11000); ++seek_calls; return 0; }
static int sceAvPlayerGetVideoData(SceAvPlayerHandle h, SceAvPlayerFrameInfo *f) { assert_live(h); (void)f; ++get_video_calls; return 0; }
static int sceGxmTextureInitLinear(int *t, void *p, int f, unsigned w, unsigned h, int m) { (void)t; (void)p; (void)f; (void)w; (void)h; (void)m; return 0; }
static void vita2d_texture_set_filters(vita2d_texture *t, int a, int b) { (void)t; (void)a; (void)b; }
'''

checks = r'''
int main(void) {
    const uint32_t handles[] = {
        0x81227d60, 0x813a8140, 0x81528200, 0x816a82c0, 0x81828380,
        0x819a8440, 0x81b285a0, 0x81ca8660, 0x81e28720, 0x81fa87e0,
        0x821288a0, 0x822a8960, 0x82428a20, 0x825a8ae0, 0x82728ba0,
        0x828a8c60, 0x82a28d20, 0x82ba8de0, 0x82d28ea0, 0x82ea8f60,
        0x83029020, 0x831a91c0, 0x83329280, 0x834a9340, 0x83629400,
        0x837a94c0, 0x83929580, 0x83aa9640, 0x12340000
    };
    assert(player == 0);
    stop_player(); update_video(); control_input(SCE_CTRL_RIGHT);
    assert(close_calls == 0 && get_video_calls == 0 && seek_calls == 0);
    for (unsigned i = 0; i < sizeof(handles)/sizeof(*handles); ++i) {
        next_handle = handles[i];
        int old_add = add_calls, old_close = close_calls;
        int old_init = init_calls, old_video = get_video_calls;
        int old_pause = pause_calls, old_resume = resume_calls, old_seek = seek_calls;
        int old_join = joined;
        start_player();
        assert(live == 1 && (uint32_t)player == next_handle);
        assert(add_calls == old_add + 1 && audio_running == 1);
        update_video(); assert(get_video_calls == old_video + 1);
        control_input(SCE_CTRL_CROSS); assert(paused && pause_calls == old_pause + 1);
        update_video(); assert(get_video_calls == old_video + 1);
        control_input(SCE_CTRL_CROSS); assert(!paused && resume_calls == old_resume + 1);
        assert(init_calls == old_init + 1); /* X must not allocate a new player. */
        control_input(SCE_CTRL_RIGHT); control_input(SCE_CTRL_LEFT);
        assert(seek_calls == old_seek + 2);
        control_input(SCE_CTRL_SQUARE);
        assert(close_calls == old_close + 1 && joined == old_join + 1);
        assert(add_calls == old_add + 2 && live == 1);
        start_player(); /* Direct restart also must release the old handle. */
        assert(close_calls == old_close + 2 && live == 1);
        stop_player(); stop_player();
        assert(close_calls == old_close + 3 && player == 0 && live == 0);
        assert(joined == old_join + 3 && !audio_running);
    }
    int old_add = add_calls, old_close = close_calls;
    next_handle = 0;
    start_player(); stop_player(); update_video();
    assert(add_calls == old_add && close_calls == old_close && player == 0);
    next_handle = 0x81227d60;
    fail_add = 1; start_player(); stop_player(); fail_add = 0;
    assert(close_calls == ++old_close && !player && !live);
    fail_create = 1; start_player(); stop_player(); fail_create = 0;
    assert(close_calls == ++old_close && !player && !live);
    fail_start = 1; start_player(); stop_player(); fail_start = 0;
    assert(close_calls == ++old_close && !player && !live && !audio_running);
    assert(stops == close_calls);
    puts("PASS: 28 logged high-bit handles + positive handle; source, pause/resume, seek, video polling, restart, idempotent cleanup, null init and failure cleanup.");
    puts("HOST API DOUBLES ONLY: no device playback or real concurrency verified.");
}
'''

with tempfile.TemporaryDirectory(prefix="tv-probe-lifecycle-") as temp:
    test = Path(temp) / "lifecycle.c"
    test.write_text(stubs + declaration + "\n" + "\n".join(
        function(name) for name in ("stop_player", "start_player", "control_input", "update_video")
    ) + checks)
    executable = Path(temp) / "lifecycle-test"
    subprocess.run([os.environ.get("CC", "cc"), "-std=c11", "-Wall", "-Wextra", "-Werror",
                    "-fsanitize=undefined", str(test), "-o", str(executable)], check=True)
    subprocess.run([str(executable)], check=True)
