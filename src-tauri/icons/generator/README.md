# Microsoft Cloud App Generator icon

Original project artwork, distinct from Microsoft's service logos:

- White cloud: the Microsoft cloud web services.
- Downward arrow and colorful app tiles: exporting independent macOS app bundles.
- Warm four-point sparkle: creating/configuring those apps.
- Rounded indigo/blue background: recognizable at Finder and Dock sizes.

The full-resolution `icon.png`, multi-resolution `icon.icns`, and local UI/runtime
PNG `dist/generator.png` are reproducibly drawn from vector paths in
`scripts/generate-generator-icon.swift` using macOS AppKit/CoreGraphics:

```bash
swift scripts/generate-generator-icon.swift
```

The ICNS includes standard and Retina representations from 16 to 1024 pixels.
The artwork has no external assets, fonts, or Microsoft logos. Regeneration is a
developer maintenance step; ordinary builds use checked-in assets and end users
need no Swift tooling.

Packaging installs this icon **only in the outer generator bundle**, before
signing. The embedded runtime and generated Teams, Outlook, Word, Excel and
PowerPoint apps retain their own service artwork. Bundle identifiers and profile
namespaces do not change.
