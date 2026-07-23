# XISO

Used by: [Microsoft Xbox (Original)](../consoles/Xbox_Overview.md)

## Purpose

XISO is an Xbox filesystem image used as a playable representation by original
Xbox emulators. Files commonly retain an `.iso` extension, but XISO is not the
same representation as a full Redump-style Xbox disc image.

## Preservation relationship

- Keep the full authoritative disc dump as the preservation master.
- Derive an XISO as a playable representation by extracting/repacking the game
  partition.
- Record the source representation and conversion tool as reproduction evidence.
- Do not label a Redump-style full-disc image as emulator-ready merely because
  it has an `.iso` extension.

## Mainstream emulator support

- xemu requires XISO images and does not directly support Redump-style full-disc
  ISOs.
- xemu does not support CHD.

## References

- [xemu disc image documentation](https://xemu.app/docs/disc-images/)
- [xemu format FAQ](https://xemu.app/docs/faq/)
