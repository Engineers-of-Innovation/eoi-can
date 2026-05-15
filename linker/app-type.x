/* Non-allocatable section holding the application type byte.
 * (INFO) strips SHF_ALLOC, so the section is present in the ELF section
 * table (and readable by the flash tool) but not part of any PT_LOAD
 * segment, so it never reaches flash on the device.
 */
SECTIONS {
  .app_type (INFO) : { KEEP(*(.app_type)) }
} INSERT AFTER .text;
