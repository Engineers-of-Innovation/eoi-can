MEMORY
{
  FLASH      (rx)  : ORIGIN = 0x08000000, LENGTH = 80K
  RAM        (rwx) : ORIGIN = 0x20000000, LENGTH = 96K
}

/* Symbols used by the bootloader to locate app partitions */
__flash_start  = ORIGIN(FLASH);
__header_start = 0x08014000;
__header_end   = 0x08014800;
__app_start    = 0x08014800;
/* Stops short of the 4K emulated-EEPROM block at 0x080FF000, so erasing the
 * app partition for a firmware update leaves the stored configuration intact. */
__app_end      = 0x080FF000;
