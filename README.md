<div align="center">

<img width="128px" src="assets/images/icon-512.png" />
<h1><b>Vleer</b></h1>

A free, open-source music app powered by the OpenMusic API Spec the open standard of music streaming. Connect all the music from different sources in one unified app.

<a href="https://docs.vleer.app">Docs</a> · <a href="https://discord.gg/invites/PEX37vvWyU">Discord</a>

</div>
<br>

![hero](https://github.com/user-attachments/assets/e0278231-226e-488a-b8d5-3c0c707a33f7)

## Features

- Native and fast, written in Rust with a GPU-rendered UI
- Song and artist matching through [OpenMusicMetadata](https://docs.vleer.app) for clean names and artwork
- No account required, telemetry covers only OS, version, and song count and is public
- Fixes messy tags, merges duplicate artists, and fills in missing years and artwork without modifying your files
- Songs identified by audio, so renaming or moving files doesn't break favorites, playlists, or history
- Fuzzy search across songs, artists, albums, and playlists
- Embedded lyrics first, then synced lyrics from LRCLIB
- Fast scanner that indexes about 60,000 songs in 10 seconds
- FLAC, MP3, OGG Vorbis, Opus, AAC, and WAV
- Loudness normalization using ReplayGain tags
- Parametric equalizer
- Last.fm scrobbling
- Discord Rich Presence
- Live SQLite library that watches your folders
- Spectrum visualizer
- Self-updating, with stable and nightly channels

## Roadmap

Not final, and nothing here is shipped yet.

- [ ] Navidrome, Subsonic, and OpenSubsonic support
- [ ] Soulseek integration
- [ ] Recommendations based on your listening habits

## Installation

<h3>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/readme/linux_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="./assets/readme/linux_light.svg">
    <img src="./assets/readme/linux_light.svg" width="24px" align="top" alt="">
  </picture>
  <span>Linux</span>
</h3>

Install using the shell script:

```sh
curl -fsSL https://raw.githubusercontent.com/vleerapp/vleer/main/scripts/install.sh | sh
```

Pass `--system` to install to `/usr/local` instead.

#### Or using AppImage:
 [ARM64](https://api.vleer.app/download/v1?os=linux&arch=aarch64&format=appimage&nightly=true) or [x64](https://api.vleer.app/download/v1?os=linux&arch=x86_64&format=appimage&nightly=true). AppImages do not self-update.

#### Or using the AUR:

```sh
yay -S vleer-git
```

#### Or using Homebrew:

```sh
brew install vleerapp/vleer/vleer # stable versions
brew install vleerapp/vleer/vleer-nightly # nightly builds
```

<h3>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/readme/apple_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="./assets/readme/apple_light.svg">
    <img src="./assets/readme/apple_light.svg" width="24px" align="top" alt="">
  </picture>
  <span>macOS</span>
</h3>

Download the DMG for [Apple silicon](https://api.vleer.app/download/v1?os=macos&arch=aarch64&nightly=true) or [Intel](https://api.vleer.app/download/v1?os=macos&arch=x86_64&nightly=true).

> [!IMPORTANT]
> 
> After copying Vleer to Applications, open Terminal and run:
>
> ```sh
> xattr -dr com.apple.quarantine /Applications/Vleer.app
> ```
>
> This is required because Vleer isn't signed yet. Otherwise, macOS will refuse to open the app.


#### Or using Homebrew:

```sh
brew install --cask vleerapp/vleer/vleer # stable versions
brew install --cask vleerapp/vleer/vleer-nightly # nightly builds
```

<h3>
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="./assets/readme/microsoft_dark.svg">
    <source media="(prefers-color-scheme: light)" srcset="./assets/readme/microsoft_light.svg">
    <img src="./assets/readme/microsoft_light.svg" width="24px" align="top" alt="">
  </picture>
  <span>Windows</span>
</h3>

Download the installer for [ARM64](https://api.vleer.app/download/v1?os=windows&arch=aarch64&nightly=true) or [x64](https://api.vleer.app/download/v1?os=windows&arch=x86_64&nightly=true).

> [!IMPORTANT]
>
> If Microsoft Defender SmartScreen appears, select **More info**, then **Run anyway**.

### Verifying downloads

Optional. Verify with [minisign](https://jedisct1.github.io/minisign/):

```sh
minisign -Vm <downloaded_file_name> -P RWRh5nLZjKooTcjP0/fbHq6xIgU9c+QhLjaVPg6gqWv1MuNRV1RWgu28
```
