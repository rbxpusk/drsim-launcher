# DELTARUNE Fight Simulator Launcher

Desktop launcher for [deltarunesim.com](https://deltarunesim.com). Downloads the game and checks every file against the site's SHA-256 manifest.

```sh
curl -fsSL https://deltarunesim.com/install.sh | sh
```

```powershell
irm https://deltarunesim.com/install.ps1 | iex
```

Build: `npm i && npx tauri build --no-bundle`

Unofficial fan project. Not affiliated with Toby Fox.
