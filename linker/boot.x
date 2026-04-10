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
__app_end      = 0x08100000;
