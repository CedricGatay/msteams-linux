# Microsoft service artwork

The existing Teams artwork remains unchanged. Outlook, Word, Excel and
PowerPoint icons are the application artwork from Microsoft Office for macOS
16.115 (`Outlook_macOS.icns`, `Word_macOS.icns`, `Excel_macOS.icns`,
`Powerpoint_macOS.icns`). The corresponding `icon.icns` files live in the
service subdirectories here.

The 128-pixel PNG renditions in `dist/services/` are shared by the generator's
local service picker and the runtime window/tray icon. They were generated from
the ICNS files with macOS `sips`, for example:

```bash
sips -s format png src-tauri/icons/word/icon.icns --out /tmp/word-icon.png
sips -z 128 128 /tmp/word-icon.png --out dist/services/word.png
```

Each generated bundle receives its service's ICNS before signing; badges are
composited onto a fresh copy of the matching runtime PNG. No installed copy of
Microsoft Office is needed to build or run the generator.

Microsoft product names and artwork are Microsoft trademarks/assets, not
original project artwork. Their inclusion identifies the web services; this
unofficial project is not affiliated with or endorsed by Microsoft and does
not claim ownership of, or grant a license to, Microsoft's artwork.
