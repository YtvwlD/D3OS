[EXTERN ___BSS_START__]
[EXTERN ___BSS_END__]
[EXTERN ___KERNEL_DATA_START__]
[EXTERN ___KERNEL_DATA_END__]
[EXTERN start]

; Multiboot2 constants
MULTIBOOT2_HEADER_MAGIC equ 0xe85250d6
MULTIBOOT2_HEADER_ARCHITECTURE equ 0
MULTIBOOT2_HEADER_LENGTH equ (multiboot2_end - multiboot2_header)
MULTIBOOT2_HEADER_CHECKSUM equ -(MULTIBOOT2_HEADER_MAGIC + MULTIBOOT2_HEADER_ARCHITECTURE + MULTIBOOT2_HEADER_LENGTH)

; Multiboot2 tag types
MULTIBOOT2_TAG_TERMINATE equ 0
MULTIBOOT2_TAG_INFORMATION_REQUEST equ 1
MULTIBOOT2_TAG_ADDRESS equ 2
MULTIBOOT2_TAG_ENTRY_ADDRESS equ 3
MULTIBOOT2_TAG_FLAGS equ 4
MULTIBOOT2_TAG_FRAMEBUFFER equ 5
MULTIBOOT2_TAG_MODULE_ALIGNMENT equ 6
MULTIBOOT2_TAG_EFI_BOOT_SERVICES equ 7
MULTIBOOT2_TAG_EFI_I386_ENTRY_ADDRESS equ 8
MULTIBOOT2_TAG_EFI_AMD64_ENTRY_ADDRESS equ 9
MULTIBOOT2_TAG_RELOCATABLE_HEADER equ 10

; Multiboot2 request types
MULTIBOOT2_REQUEST_BOOT_COMMAND_LINE equ 1
MULTIBOOT2_REQUEST_BOOT_LOADER_NAME equ 2
MULTIBOOT2_REQUEST_MODULE equ 3
MULTIBOOT2_REQUEST_BASIC_MEMORY_INFORMATION equ 4
MULTIBOOT2_REQUEST_BIOS_BOOT_DEVICE equ 5
MULTIBOOT2_REQUEST_MEMORY_MAP equ 6
MULTIBOOT2_REQUEST_VBE_INFO equ 7
MULTIBOOT2_REQUEST_FRAMEBUFFER_INFO equ 8
MULTIBOOT2_REQUEST_ELF_SYMBOLS equ 9
MULTIBOOT2_REQUEST_APM_TABLE equ 10
MULTIBOOT2_REQUEST_EFI_32_BIT_SYSTEM_TABLE_POINTER equ 11
MULTIBOOT2_REQUEST_EFI_64_BIT_SYSTEM_TABLE_POINTER equ 12
MULTIBOOT2_REQUEST_SMBIOS_TABLES equ 13
MULTIBOOT2_REQUEST_ACPI_OLD_RSDP equ 14
MULTIBOOT2_REQUEST_ACPI_NEW_RSDP equ 15
MULTIBOOT2_REQUEST_NETWORKING_INFORMATION equ 16
MULTIBOOT2_REQUEST_EFI_MEMORY_MAP equ 17
MULTIBOOT2_REQUEST_EFI_BOOT_SERVICES_NOT_TERMINATED equ 18
MULTIBOOT2_REQUEST_EFI_32_BIT_IMAGE_HANDLE_POINTER equ 19
MULTIBOOT2_REQUEST_EFI_64_BIT_IMAGE_HANDLE_POINTER equ 20
MULTIBOOT2_REQUEST_IMAGE_LOAD_BASE_PHYSICAL_ADDRESS equ 21

; Multiboot2 tag flags
MULTIBOOT2_TAG_FLAG_REQUIRED equ 0x00
MULTIBOOT2_TAG_FLAG_OPTIONAL equ 0x01

; Multiboot2 console flags
MULTIBOOT2_CONSOLE_FLAG_FORCE_TEXT_MODE equ 0x01
MULTIBOOT2_CONSOLE_FLAG_SUPPORT_TEXT_MODE equ 0x02

; Multiboot2 framebuffer options
MULTIBOOT2_GRAPHICS_MODE   equ 0
MULTIBOOT2_GRAPHICS_WIDTH  equ 800
MULTIBOOT2_GRAPHICS_HEIGHT equ 600
MULTIBOOT2_GRAPHICS_BPP    equ 32

;
; boot_bp.asm - Constants
;

; Address of boot code for application processors
; Also defined in 'consts.rs'. Must be consistent!
RELOCATE_BOOT_CODE: equ 0x40000

; Stack
STACK_MEM_SIZE: equ  65536	; total size of all stacks
STACK_SIZE: equ 4096		; stack size of one processor

; 254 GB max. supported DRAM size by paging tables
MAX_MEM: equ 254

; Speicherplatz fuer die Seitentabelle

[GLOBAL pagetable_start]
pagetable_start:  equ 0x103000

[GLOBAL pagetable_end]
pagetable_end:  equ 0x200000

; In 'linker.ld'
[EXTERN ___BOOT_AP_START__]
[EXTERN ___BOOT_AP_END__]

[GLOBAL copy_boot_code] ; wird in 'startup.rs' benoetigt
[GLOBAL RELOCATE_BOOT_CODE] ; wird in 'startup.rs' benoetigt


[SECTION .multiboot2_sec]
; boot.asm
[BITS 64]
multiboot2_header:
    ; Header
    align 8
    dd MULTIBOOT2_HEADER_MAGIC
    dd MULTIBOOT2_HEADER_ARCHITECTURE
    dd MULTIBOOT2_HEADER_LENGTH
    dd MULTIBOOT2_HEADER_CHECKSUM

    ; EFI amd64 entry address tag
    align 8
    dw MULTIBOOT2_TAG_EFI_AMD64_ENTRY_ADDRESS
    dw MULTIBOOT2_TAG_FLAG_REQUIRED
    dd 12
    dd (boot)

    ; EFI boot services tag
    align 8
    dw MULTIBOOT2_TAG_EFI_BOOT_SERVICES
    dw MULTIBOOT2_TAG_FLAG_REQUIRED
    dd 8

    ; Information request tag (required)
    align 8
    dw MULTIBOOT2_TAG_INFORMATION_REQUEST
    dw MULTIBOOT2_TAG_FLAG_REQUIRED
    dd 44
    dd MULTIBOOT2_REQUEST_BOOT_COMMAND_LINE
    dd MULTIBOOT2_REQUEST_MODULE
    dd MULTIBOOT2_REQUEST_MEMORY_MAP
    dd MULTIBOOT2_REQUEST_FRAMEBUFFER_INFO
    dd MULTIBOOT2_REQUEST_ACPI_OLD_RSDP
    dd MULTIBOOT2_REQUEST_ACPI_NEW_RSDP
    dd MULTIBOOT2_REQUEST_EFI_BOOT_SERVICES_NOT_TERMINATED
    dd MULTIBOOT2_REQUEST_EFI_64_BIT_SYSTEM_TABLE_POINTER
    dd MULTIBOOT2_REQUEST_EFI_64_BIT_IMAGE_HANDLE_POINTER

    ; Information request tag (optional)
    align 8
    dw MULTIBOOT2_TAG_INFORMATION_REQUEST
    dw MULTIBOOT2_TAG_FLAG_OPTIONAL
    dd 20
    dd MULTIBOOT2_REQUEST_BOOT_LOADER_NAME
    dd MULTIBOOT2_REQUEST_EFI_BOOT_SERVICES_NOT_TERMINATED
    dd MULTIBOOT2_REQUEST_EFI_MEMORY_MAP

    ; Framebuffer tag
    align 8
    dw MULTIBOOT2_TAG_FRAMEBUFFER
    dw MULTIBOOT2_TAG_FLAG_REQUIRED
    dd 20
    dd MULTIBOOT2_GRAPHICS_WIDTH
    dd MULTIBOOT2_GRAPHICS_HEIGHT
    dd MULTIBOOT2_GRAPHICS_BPP

    ; Module alignment tag
    align 8
    dw MULTIBOOT2_TAG_MODULE_ALIGNMENT
    dw MULTIBOOT2_TAG_FLAG_OPTIONAL
    dd 8

    ; Termination tag
    align 8
    dw MULTIBOOT2_TAG_TERMINATE
    dw MULTIBOOT2_TAG_FLAG_REQUIRED
    dd 8
multiboot2_end:

[SECTION .text]

boot:
    cld ; Expected by GCC
    cli ; Disable interrupts

    ; Clear BSS section
    mov rdi, ___BSS_START__
clear_bss:
    mov byte [rdi], 0
    inc rdi
    cmp rdi, ___BSS_END__
    jne clear_bss

    ; Switch stack to our own stack, because the EFI stack may be located inside
    ; reserved memory and will thus be ignored by our paging implementation.
    mov rsp, init_stack.end

    ; Call rust function with multiboot2 magic number and address (initially located in eax and ebx)
    xor rdi, rdi
    xor rsi, rsi
    mov edi, eax
    mov esi, ebx
    call start

;
; boot_bp.asm - Code
;
;
; 32 bit entry function for bootstrap processor and later called by
; application processors from 'boot_ap.asm'
; Sets up GDT and a stack
;
[BITS 32]
startup:
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
	mov    eax, STACK_SIZE
	lock xadd [stack_mem_ptr], eax
	; Stack grows downwards, thus we need to add STACK_SIZE again
	add eax, STACK_SIZE
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
	;jmp    2 * 0x8 : longmode_start (version of this code in dpmos)
	jmp    2 * 0x8 : longmode_start_ap


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
	; call   setup_idt

	; Pruefen, ob es sich um den Bootstrap-Core oder einen weiteren
	; Application-Core handelt
	mov eax, [is_bootstrap]
	cmp eax, 0
	jne longmode_start_ap

	mov rax, is_bootstrap
	mov dword [rax], 1

	; Init PICs
	; call   reprogram_pics

    ; Call Rust entry function in 'startup.rs' for bootstrap processor
    ; extern startup
    ; call startup
	jmp end

longmode_start_ap:
    extern setupIdt
    call setupIdt
    ; Call Rust entry function in 'boot.rs' for application proc.
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
; boot_ap section
;
;
; The Segment '.boot_seg_ap' will be relocated by the bootstrap proc.
; grub loads the code above 1 MB but real mode code needs to be <1 MB
;
[SECTION .boot_seg_ap]

; Variables need to be modified during code relocation in 'bootbp.asm'
[GLOBAL gdt_ap]
[GLOBAL gdtd_ap]

; Entry function of bootstrap processor in 'boot_bp.asm'
;[EXTERN startup]


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
	jmp startup

    ; print `**` to screen
    ; should never been reached
    mov dword [0xb8000], 0x2e262e26
	hlt



[SECTION .data]
;
;boot_bp.asm
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
;
;boot.asm
;
global init_stack:data (init_stack.end - init_stack)
init_stack:
	  resb STACK_SIZE
.end:



;
; boot_bp.asm
;
pml4:
	resb   4096
	alignb 4096

pdp:
	resb   MAX_MEM*8
	alignb 4096

pd:
	resb   MAX_MEM*4096

reserve_stack_mem:
	resb STACK_MEM_SIZE
.end: