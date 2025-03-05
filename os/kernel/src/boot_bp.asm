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

global start_asm

;
; Constants
;

; Address of boot code for application processors
; Also defined in 'consts.rs'. Must be consistent!
RELOCATE_BOOT_CODE: equ 0x40000

; Stack
STACK_MEM_SIZE: equ  65536	; total size of all stacks
STACK_SIZE_ONE: equ 4096		; stack size of one processor

; 254 GB max. supported DRAM size by paging tables
MAX_MEM: equ 254

; Speicherplatz fuer die Seitentabelle

[GLOBAL pagetable_start]
pagetable_start:  equ 0x103000

[GLOBAL pagetable_end]
pagetable_end:  equ 0x200000

; In 'interrupts.asm'
[EXTERN setup_idt]
[EXTERN reprogram_pics]

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
; This function is called from 'startup.rs' gerufen for relocating the
; boot code in 'boot_ap.asm' for the application processors below 1 MB
; Necessary because the application processors start in real mode
;
copy_boot_code:
	mov rax, RELOCATE_BOOT_CODE
	mov rbx, ___BOOT_AP_START__   ; = 0x100020
	mov rcx, ___BOOT_AP_END__     ; = 0x100076
copyc:
	mov rdx, [rbx]    ; load qword from source
	mov [rax], rdx    ; write qword to destination
	add rbx, 8        ; next qword of source 
	add rax, 8        ; next qword of destination 
	cmp rbx, rcx      ; check if end is reached
	jle copyc

	; Adresse von 'gdt_ap' im kopierten Code
	mov rbx, gdt_ap
	sub rbx, ___BOOT_AP_START__
    add rbx, RELOCATE_BOOT_CODE
    
	; Adresse von 'gdtd_ap' im kopierten Code
	mov rax, gdtd_ap
	sub rax, ___BOOT_AP_START__
    add rax, RELOCATE_BOOT_CODE
    add rax, 2     ; skip 'limit'

    ; Aktualisieren von 'gdtd_ap' im kopierten Code
    mov [eax], ebx
	ret
	

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
	
