;**********************************************************************
;*                                                                    *
;*                  B O O T _ A P                                     *
;*                                                                    *
;*--------------------------------------------------------------------*
;* Description:     This is the boot code for all application proc-   *
;*                  essors. The boostrap processor sends in an IPI    *
;*                  = Inter-Processor-Interrupt (IPI) in 'startup.rs' * 
;*                  to all application processors with the addess of  * 
;*                  the function 'boot_ap'.                           *
;*                  All application processors start their execution  * 
;*                  in real mode and we need to switch to protected   *
;*                  mode first. Afterwards we switch to long mode by  * 
;*                  calling 'start' in 'boot_bp.asm' which is the     * 
;*                  entry function for the boot processor. The latter *
;*                  is switched to protected mode by grub.            *                                      * 
;*                                                                    *
;* Autor:           Michael Schoettner, Univ. Duesseldorf, 25.8.2022  *
;**********************************************************************

; 
; The Segment '.boot_seg_ap' will be relocated by the bootstrap proc.
; grub loads the code above 1 MB but real mode code needs to be <1 MB
;
[SECTION .boot_seg_ap]

; Variables need to be modified during code relocation in 'bootbp.asm' 
[GLOBAL gdt_ap]
[GLOBAL gdtd_ap]

; Entry function of bootstrap processor in 'boot_bp.asm'
[EXTERN start_asm]

 
; 'boot_ap' is the entry function for all application processors
USE16

boot_ap:
	; Initialize segment registers
	mov ax, cs 		; Same segment for code and data
	mov ds, ax	 	; For the time being we need no stack

	cli				
	mov al, 0x80
	out 0x70, al   	; disable NMI 

	; Set GDT
	lgdt [gdtd_ap - boot_ap]

	; Switch to protected mode
	mov eax, cr0 	; Set PM bit in control register cr0
	or  eax, 1
	mov cr0, eax

	; Far jump to load cs
	jmp dword 0x08:boot_ap32


; GDT for application processors (will be replaced later)
; Attentation: This GDT is in the segment '.boot_seg_ap' because it 
; will be copied below 1 MB during code relocation
align 4

gdt_ap:
	dw  0,0,0,0   ; NULL descriptor

	; 32 bit code segment deskriptor
	dw  0xFFFF    ; 4Gb - (0x100000*0x1000 = 4Gb)
	dw  0x0000    ; base address=0
	dw  0x9A00    ; code read/exec
	dw  0x00CF    ; granularity=4096, 386 (+5th nibble of limit)

	; 32 bit data segment deskriptor
	dw  0xFFFF    ; 4Gb - (0x100000*0x1000 = 4Gb)
	dw  0x0000    ; base address=0
	dw  0x9200    ; data read/write
	dw  0x00CF    ; granularity=4096, 386 (+5th nibble of limit)

; Limit and address of GDT, needed to load GDT
gdtd_ap:
	dw  3*8 - 1   ; GDT Limit=24, 4 GDT entries - 1
	dd  gdt_ap    ; Adress of GDT


[SECTION .text]

USE32

; 32 bit protected mode from here
; setting up segment registers and call 'start' in 'boot_bp.asm'
boot_ap32:
	mov ax, 0x10
	mov ds, ax
	mov es, ax
	mov fs, ax
	mov gs, ax
	mov ss, ax

	; In 'boot_bp.asm"
	jmp start_asm
	
    ; print `**` to screen
    ; should never been reached
    mov dword [0xb8000], 0x2e262e26
	hlt

