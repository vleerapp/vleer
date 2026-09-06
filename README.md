<div align="center">

<img width="128px" src="assets/images/icon.png" />
<h1><b>Vleer</b></h1>

A free, open-source music app powered by the OpenMusic API Spec the open standard of music streaming. Connect all the music from different sources in one unified app.

<a href="https://docs.vleer.app">Docs</a> · <a href="https://discord.gg/invites/PEX37vvWyU">Discord</a>

</div>
<br>

![hero](https://github.com/user-attachments/assets/418403ed-e5ff-412b-89ac-6fb501de79ab)

## Roadmap

[Tracker](https://github.com/orgs/vleerapp/projects/4)

- [ ] Stable local music player
- [ ] OpenMusic API integration

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

This installs to `~/.local` without root and keeps itself up to date from inside the app. Pass `--system` to install to `/usr/local` instead.

An AppImage is also available for [ARM64](https://api.vleer.app/downloads/v1?os=linux&arch=aarch64&format=appimage&nightly=true) or [x64](https://api.vleer.app/downloads/v1?os=linux&arch=x86_64&format=appimage&nightly=true). AppImages do not self-update.

Install from the AUR:

```sh
yay -S vleer-git
```

Or via Homebrew:

```sh
brew install vleerapp/vleer/vleer # stable versions
```

```sh
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

Download the DMG for [Apple silicon](https://api.vleer.app/downloads/v1?os=macos&arch=aarch64&nightly=true) or [Intel](https://api.vleer.app/downloads/v1?os=macos&arch=x86_64&nightly=true).

After copying Vleer to Applications, open Terminal and run:

```sh
xattr -dr com.apple.quarantine /Applications/Vleer.app
```

This is required because Vleer isn't signed yet. Otherwise, macOS will refuse to open the app.

Or via Homebrew (handles the quarantine step automatically):

```sh
brew install --cask vleerapp/vleer/vleer # stable versions
```

```sh
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

Download the installer for [ARM64](https://api.vleer.app/downloads/v1?os=windows&arch=aarch64&nightly=true) or [x64](https://api.vleer.app/downloads/v1?os=windows&arch=x86_64&nightly=true).

If Microsoft Defender SmartScreen appears, select **More info**, then **Run anyway**.

### Verifying downloads

Optional. Import the signing key (fingerprint `7E48 1786 6409 4A19 EF60  EEC8 8524 0717 1261 C8A4`):

```sh
curl -sSL https://raw.githubusercontent.com/vleerapp/vleer/main/assets/key.asc | gpg --import
gpg --verify <downloaded_file_name>.sig <downloaded_file_name>
```
