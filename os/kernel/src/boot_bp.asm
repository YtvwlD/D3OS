;**********************************************************************
;*                                                                    *
;*                  B O O T _ B P                                     *
;*                                                                    *
;*--------------------------------------------------------------------*
;* Description:     This is the boot code for all processors. 'start' *
;*                  is the 32 bit protected mode code entry function  * 
;*                  called by grub for the bootstrap processor. It    *
;*                  will setup the GDT, page tables, a stack, switch  * 
;*                  to long mode and call 'startup', the first Rust   * 
;*                  function.                                         *
;*                                                                    *
;*                  The bootstrap processor will do further init-     *
;*                  ialization in Rust and relocated the boot code    *
;*                  (in 'boot_ap.asm') for application processors     *
;*                  using 'copy_boot_code'. Application processors    *
;*                  will start executing in 'boot_ap.asm' and then    *
;*                  call 'start' here to switch to long mode and get  *
;*                  a stack. Finally, The Rust function 'startup_ap'  * 
;*                  is called.                                        * 
;*                                                                    *
;*                  We reserve 64 KB for all stacks. Each processor   * 
;*                  gets 4 KB for its stack. This will limit the      *
;*                  number of supported cores.                        *
;*                                                                    *
;* Autor:           Michael Schoettner, Univ. Duesseldorf, 25.8.2022  *
;**********************************************************************

; Stack

; 254 GB max. supported DRAM size by paging tables
MAX_MEM: equ 254
STACK_MEM_SIZE: equ  6553600	; total size of all stacks (16)
STACK_SIZE_ONE: equ 409600		; stack size of one processor

; Speicherplatz fuer die Seitentabelle

[GLOBAL pagetable_start]
pagetable_start:  equ 0x103000

[GLOBAL pagetable_end]
pagetable_end:  equ 0x200000

; In 'boot.rs' - hier starten die AP's
[extern startup_ap]
; In 'interrupt_dispatcher.rs'
[EXTERN setup_ap_idt]

global start_asm

; In 'linker.ld'
[EXTERN ___BOOT_AP_START__]     
[EXTERN ___BOOT_AP_END__]     

; In 'boot.asm', benoetigt beim Umkopieren dieses Boot-Codes fuer die APs
[EXTERN gdt_ap]
[EXTERN gdtd_ap]

[GLOBAL copy_boot_code] ; wird in 'startup.rs' benoetigt
[GLOBAL RELOCATE_BOOT_CODE] ; wird in 'startup.rs' benoetigt


[SECTION .text]

;
; 32 bit entry function for bootstrap processor and later called by 
; application processors from 'boot_ap.asm'
; Sets up GDT and a stack
;
[BITS 32]
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
	call set_cr3

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

set_cr3:
    mov eax, 0x1000 ;TODO: get actual variable in here
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
    ; Sets the idt via setup_ap_idt in interrupt_dispatcher.rs
	call setup_ap_idt

    ; Call Rust entry function in 'startup.rs' for bootstrap processor
    ;extern startup
    ;call startup
	jmp end

longmode_start_ap:
    ; Call Rust entry function in 'startup.rs' for application proc.
    call startup_ap

end:
	; We should never end up here
    cli
    mov dword [0xb8000], 0x2f2a2f2a
	hlt


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
	
