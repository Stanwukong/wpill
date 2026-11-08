{{WHATS_NEW}}

## Install

Download `Cay_{{VERSION}}_x64-setup.exe` below and run it. It's for Windows 11 (64-bit) and installs for your user only,
with no admin rights. Quit any older Cay or TopBar from its tray first.

Cay isn't code-signed yet, so Windows SmartScreen says "Windows protected your PC" and lists the publisher as unknown.
Check your download first (below), then click **More info**, then **Run anyway**.

## Check your download

The installer's SHA-256 checksum:

```
{{SHA256}}
```

In PowerShell, in the folder you downloaded it to:

```powershell
Get-FileHash .\Cay_{{VERSION}}_x64-setup.exe -Algorithm SHA256
```

The hash must match the one above exactly. If it doesn't, delete the file.

## What to try

This is a pre-release, so these are the setups we know least about:

- More than one monitor, and changing which one is the main one.
- Display scaling above 100% (Settings > System > Display > Scale).
- Dragging a window to the top centre of the screen, where Windows 11 shows Snap Layouts.
- Fullscreen games and fullscreen video: the island should get out of the way.
- On a laptop: brightness keys, plugging in and unplugging the charger, low battery.
- Music from Spotify, and from a browser (YouTube or similar).

## Known problems and bug reports

- [Known limitations](https://github.com/Stanwukong/wpill#known-limitations) lists what doesn't work yet.
- Found something else? [Report a bug](https://github.com/Stanwukong/wpill/issues/new?template=bug_report.yml).
