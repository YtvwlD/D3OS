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
; Constants
;

; Stack
STACK_MEM_SIZE: equ  65536	; total size of all stacks
STACK_SIZE_ONE: equ 4096		; stack size of one processor

; 254 GB max. supported DRAM size by paging tables
MAX_MEM: equ 254

; this is real-mode code
[SECTION .boot_seg_ap exec]

; Variables need to be modified during code relocation in 'bootbp.asm' 
[GLOBAL gdt_ap]
[GLOBAL gdtd_ap]

[EXTERN setup_idt]

 
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
; is needed in real-mode
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

start_asm:
	cld              ; required by GCC; Rust?
	cli              
	lgdt   [gdt_80]  ; load GDT

	; Set segment registers
	mov    eax, 3 * 0x8
	mov    ds, ax
	mov    es, ax
	mov    fs, ax
	mov    gs, ax
	mov    ss, ax

	; Init stack
	mov    eax, STACK_SIZE_ONE
	lock xadd [stack_mem_ptr], eax
	; Stack grows downwards, thus we need to add STACK_SIZE again
	add eax, STACK_SIZE_ONE
	mov    esp, eax

	jmp    init_longmode



;
;  Switch to long mode
;
init_longmode:
	; Activate page address extensions
	mov    eax, cr4
	or     eax, 1 << 5
	mov    cr4, eax

	; Setup paging tables (required for long mode)
	call   setup_paging

	; Switch to long mode (for the time being in compatibility mode)
	mov    ecx, 0x0C0000080 ; Select Extended Feature Enable Register 
	rdmsr
	or     eax, 1 << 8 		; LME (Long Mode Enable)
	wrmsr

	; Activate paging
	mov    eax, cr0
	or     eax, 1 << 31
	mov    cr0, eax

	; Jump to 64 bit code segment (activates full long mode)
	jmp    2 * 0x8 : longmode_start


;
; Create page tables for long mode using 2 MB pages and a 1:1 mapping 
; for 0 - MAX_MEM physical memory
;
setup_paging:
	; PML4 (Page Map Level 4)
	mov    eax, pdp
	or     eax, 0xf
	mov    dword [pml4+0], eax
	mov    dword [pml4+4], 0

	; PDPE (Page-Directory-Pointer Entry) for 16 GB
	mov    eax, pd
	or     eax, 0x7           ; Address of first table including flags
	mov    ecx, 0
fill_tables2:
	cmp    ecx, MAX_MEM       ; Referencing MAX_MEM tables 
	je     fill_tables2_done
	mov    dword [pdp + 8*ecx + 0], eax
	mov    dword [pdp + 8*ecx + 4], 0
	add    eax, 0x1000        ; Each table has 4 KB size
	inc    ecx
	ja     fill_tables2
fill_tables2_done:

	; PDE (Page Directory Entry)
	mov    eax, 0x0 | 0x87    ; Start address byte 0..3 (=0) + flags
	mov    ebx, 0             ; Start address byte 4..7 (=0)
	mov    ecx, 0
fill_tables3:
	cmp    ecx, 512*MAX_MEM  ; Fill MAX_MEM tables each with 512 entries
	je     fill_tables3_done
	mov    dword [pd + 8*ecx + 0], eax ; low bytes
	mov    dword [pd + 8*ecx + 4], ebx ; high bytes
	add    eax, 0x200000     ; 2 MB for each page
	adc    ebx, 0            ; Overflow? -> increment high part of addr
	inc    ecx
	ja     fill_tables3
fill_tables3_done:

	; Set base pointer to PML4
	mov    eax, pml4
	mov    cr3, eax
	ret


;
; Long mode entry function
;
; The bootstrap processor will setup the IDT and reprogram PICs and the
; call 'startup', the first Rust function
;
; is_bootstrap' allows to identify applications processors which will 
; init their IDT but will not touch the PICs
;
longmode_start:
[BITS 64]
	call   setup_idt

	; Pruefen, ob es sich um den Bootstrap-Core oder einen weiteren 
	; Application-Core handelt
	;mov eax, [is_bootstrap]
	;cmp eax, 0
	;jne longmode_start_ap
	jmp longmode_start_ap

	mov rax, is_bootstrap
	mov dword [rax], 1
	
	; Init PICs
	;call   reprogram_pics

    ; Call Rust entry function in 'startup.rs' for bootstrap processor
    ;extern startup
    ;call startup
	jmp end

longmode_start_ap:
    ; Call Rust entry function in 'startup.rs' for application proc.
    extern startup_ap
    call startup_ap

end:
	; We should never end up here
    cli
    mov dword [0xb8000], 0x2f2a2f2a
	hlt	

;	
;  Memory for page tables
;
[SECTION .bss]

[GLOBAL pml4]
[GLOBAL pdp]
[GLOBAL pd]

pml4:
	resb   4096
	alignb 4096

pdp:
	resb   MAX_MEM*8
	alignb 4096

pd:
	resb   MAX_MEM*4096



[SECTION .data]

;
; GDT 
;
gdt:
	dw  0,0,0,0  ; NULL-Deskriptor

	; 32-Bit-Codesegment-Deskriptor
	dw  0xFFFF   ; 4Gb - (0x100000*0x1000 = 4Gb)
	dw  0x0000   ; base address=0
	dw  0x9A00   ; code read/exec
	dw  0x00CF   ; granularity=4096, 386 (+5th nibble of limit)

	; 64-Bit-Codesegment-Deskriptor
	dw  0xFFFF   ; 4Gb - (0x100000*0x1000 = 4Gb)
	dw  0x0000   ; base address=0
	dw  0x9A00   ; code read/exec
	dw  0x00AF   ; granularity=4096, 386 (+5th nibble of limit),long mode

	; Datensegment-Deskriptor
	dw  0xFFFF   ; 4Gb - (0x100000*0x1000 = 4Gb)
	dw  0x0000   ; base address=0
	dw  0x9200   ; data read/write
	dw  0x00CF   ; granularity=4096, 386 (+5th nibble of limit)

gdt_80:
	dw  4*8 - 1  ; GDT Limit=24, 4 GDT Eintraege - 1
	dq  gdt      ; Adresse der GDT

;
; Global variable needed to detect the boostrap processor
;
is_bootstrap:
	dq	0	

;	
;  Memory for stacks
;
stack_mem_ptr:
	dd  reserve_stack_mem 

[SECTION .bss]
reserve_stack_mem:
	resb STACK_MEM_SIZE
.end:
	
