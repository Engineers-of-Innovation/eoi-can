MEMORY
{
  BOOT       (rx)  : ORIGIN = 0x08000000, LENGTH = 80K   /* Bootloader - do not use */
  HEADER     (r)   : ORIGIN = 0x08014000, LENGTH = 2K    /* App header - do not use */
  FLASH      (rx)  : ORIGIN = 0x08014800, LENGTH = 942K  /* Application code */
  RAM        (rwx) : ORIGIN = 0x20000000, LENGTH = 96K
}
