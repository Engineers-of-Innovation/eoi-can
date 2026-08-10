MEMORY
{
  FLASH      (rx)  : ORIGIN = 0x08000000, LENGTH = 1020K /* Application code (no bootloader) */
  CONFIG     (r)   : ORIGIN = 0x080FF000, LENGTH = 4K    /* Emulated EEPROM - do not use */
  RAM        (rwx) : ORIGIN = 0x20000000, LENGTH = 96K
}
