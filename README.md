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

## Installation/Testing

<details>
<summary>Linux</summary>

Download the latest nightly build [here](https://github.com/vleerapp/vleer/actions/workflows/nightly.yml).

**Arch Linux:** 
```bash
yay -S vleer-git
```

(Optional) Verify downloads with GPG. Import the signing key (fingerprint `7E48 1786 6409 4A19 EF60  EEC8 8524 0717 1261 C8A4`):

```bash
curl -sSL https://raw.githubusercontent.com/vleerapp/vleer/main/assets/key.asc | gpg --import
gpg --verify <downloaded_file_name>.sig <downloaded_file_name>
```

</details>

<details>
<summary>Windows</summary>

Download the latest nightly build [here](https://github.com/vleerapp/vleer/actions/workflows/nightly.yml).

SmartScreen may block the installer. Click "More info" then "Run anyway".

(Optional) Verify downloads with GPG. Import the signing key (fingerprint `7E48 1786 6409 4A19 EF60  EEC8 8524 0717 1261 C8A4`):

```bash
curl -sSL https://raw.githubusercontent.com/vleerapp/vleer/main/assets/key.asc | gpg --import
gpg --verify <downloaded_file_name>.sig <downloaded_file_name>
```

</details>

<details>
<summary>macOS</summary>

Download the latest nightly build [here](https://github.com/vleerapp/vleer/actions/workflows/nightly.yml).

You may see a warning that the app is damaged. Run `xattr -dr com.apple.quarantine /Applications/Vleer.app` to fix it.

(Optional) Verify downloads with GPG. Import the signing key (fingerprint `7E48 1786 6409 4A19 EF60  EEC8 8524 0717 1261 C8A4`):

```bash
curl -sSL https://raw.githubusercontent.com/vleerapp/vleer/main/assets/key.asc | gpg --import
gpg --verify <downloaded_file_name>.sig <downloaded_file_name>
```

</details>
