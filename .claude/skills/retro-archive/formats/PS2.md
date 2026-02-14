# Sony PlayStation 2 DVD Format
## File Extensions
- `.iso` - ISO 9660 DVD image
- `.bin/.cue` - Binary image with cue sheet (for CD games)
- `.chd` - Compressed format

## Disc Format Structure

PlayStation 2 uses standard DVD format with ISO 9660 filesystem, similar to PS1 but on DVD media.

| Area | Description |
|------|-------------|
| System Area | Boot information and region data |
| Volume Descriptors | ISO 9660 filesystem structure |
| Data Area | Game files, executables, assets |

## Detection Method

1. Check for ISO 9660 volume descriptor
2. Look for PlayStation 2 system identifier
3. Verify DVD format compliance
4. Check for PS2 executable files (.ELF format)
5. Region protection similar to PS1 but DVD-based

## File System

- **Standard:** ISO 9660 with extensions
- **Sector Size:** 2048 bytes (DVD standard)
- **Capacity:** Up to 4.7GB (single layer) or 8.5GB (dual layer)

## Boot Process

1. System reads boot information from disc
2. Loads main executable (typically SYSTEM.CNF points to main .ELF)
3. Region checking performed by BIOS
4. Game execution begins

## Regional Protection

Similar to PS1, PS2 uses region coding and copy protection, but implemented through DVD format rather than CD wobble.

## Sources

- [PlayStation 2 Technical Specifications](https://en.wikipedia.org/wiki/PlayStation_2_technical_specifications)
- [Copetti PS2 Architecture Analysis](https://www.copetti.org/writings/consoles/playstation-2/)

