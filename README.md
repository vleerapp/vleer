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

(Optional) Verify downloads with [minisign](https://jedisct1.github.io/minisign/):

```bash
minisign -Vm <downloaded_file_name> -P RWQc0Dzx5Dhao5YtQGj79Y4AN7U1pjJFctj3dCLr4tQqkjewjl5xnSqe
```

</details>

<details>
<summary>Windows</summary>

Download the latest nightly build [here](https://github.com/vleerapp/vleer/actions/workflows/nightly.yml).

SmartScreen may block the installer. Click "More info" then "Run anyway".

(Optional) Verify downloads with [minisign](https://jedisct1.github.io/minisign/):

```bash
minisign -Vm <downloaded_file_name> -P RWQc0Dzx5Dhao5YtQGj79Y4AN7U1pjJFctj3dCLr4tQqkjewjl5xnSqe
```

</details>

<details>
<summary>macOS</summary>

Download the latest nightly build [here](https://github.com/vleerapp/vleer/actions/workflows/nightly.yml).

You may see a warning that the app is damaged. Run `xattr -dr com.apple.quarantine /Applications/Vleer.app` to fix it.

(Optional) Verify downloads with [minisign](https://jedisct1.github.io/minisign/):

```bash
minisign -Vm <downloaded_file_name> -P RWQc0Dzx5Dhao5YtQGj79Y4AN7U1pjJFctj3dCLr4tQqkjewjl5xnSqe
```

</details>
