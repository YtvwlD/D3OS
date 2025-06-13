ENTRY(start)

SECTIONS {
    . = 1M;   /* load at address 1MB */

    ___KERNEL_DATA_START__ = .;

    .boot :
    {
        /* ensure that the multiboot header is at the beginning */
        *(.multiboot2_sec)
    }

    /* Startcode fuer die APs, wird von System::init() reloziert */
	.boot_seg_ap ALIGN(0x10) :
	{
		___BOOT_AP_START__ = .;
		*(".boot_seg_ap")
		*(".boot_seg_ap$")
	}
	___BOOT_AP_END__ = .;


    .text :
    {
        *(.text)
    }

   .bss : 
    {
      ___BSS_START__ = .;
      *(".bss")
      *(".bss.*")
      ___BSS_END__ = .;
    }

    ___KERNEL_DATA_END__ = .;
}
