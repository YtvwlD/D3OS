ENTRY(start)

SECTIONS {
    /*
     * load the kernel to the following address
     *
     * a QEMU VM with OVMF roughly has the following memory layout:
     *   0KB to 640KB: free
     * 640KB to 1MB:   devices
     *   1MB to 8MB:   free
     *   8MB to 9MB:   ACPI
     *   9MB to 24MB:  Boot Services
     */
    . = 25M;

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

    .text ALIGN (4K) :
    {
        *(.text*)
    }

   .bss ALIGN (4K) :
    {
      ___BSS_START__ = .;
      *(".bss*")
      ___BSS_END__ = .;
    }

    .data ALIGN (4K) :
    {
        *(.data*)
    }
    ___KERNEL_DATA_END__ = .;
}
