# App icons

These are generated, not checked in. From `clients/desktop/`:

```bash
npm install
npm run icon          # = tauri icon ../../thymosG.PNG
```

`tauri icon` produces every size the bundler needs (`32x32.png`,
`128x128.png`, `128x128@2x.png`, `icon.icns`, `icon.ico`) into this directory.
`tauri.conf.json` already references them.
