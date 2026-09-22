/* Augment, rather than replace, the VitaSDK default linker script.
 * .init_array is the first section in this application's writable PT_LOAD.
 * Moving the location counter OUTSIDE an output section leaves a virtual gap
 * for vita-elf-create's SCE metadata without growing the executable PT_LOAD.
 * Older VitaSDK images do not implement the __sce_headroom symbol hook.
 */
SECTIONS
{
  . = ALIGN(0x10000) + 0x10000;
}
INSERT BEFORE .init_array;
