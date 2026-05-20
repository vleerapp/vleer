<div align="center">

<img width="128px" src="assets/images/icon.png" />
<h1><b>Vleer</b></h1>

A free, open-source music app powered by the OpenMusic API Spec—the open standard of music streaming. Access your local library, self-hosted servers, or any compatible source, all without subscriptions.

<a href="https://docs.vleer.app">Docs</a> · <a href="https://discord.gg/invites/PEX37vvWyU">Discord</a>

</div>
<br>

![hero](https://github.com/user-attachments/assets/418403ed-e5ff-412b-89ac-6fb501de79ab)

## Roadmap

[Tracker](https://github.com/orgs/vleerapp/projects/4)

- [ ] Stable local music player
- [ ] OpenMusic API integration

## Installation

> [!IMPORTANT]
> Currently there are no releases out yet since Vleer is still being developed so this step can be ignored. If you want to try it out download the latest nightly build [here](https://github.com/vleerapp/vleer/actions/workflows/nightly.yml) just dont expect everything to work flawlessly. 

All releases are signed with [minisign](https://jedisct1.github.io/minisign/). To verify a download:

```bash
minisign -Vm <file> -P RWQc0Dzx5Dhao5YtQGj79Y4AN7U1pjJFctj3dCLr4tQqkjewjl5xnSqe
```

> [!NOTE]
> **macOS:** You may see a warning that the app is damaged. Run `xattr -dr com.apple.quarantine /Applications/Vleer.app` to fix it.

> [!NOTE]
> **Windows:** SmartScreen may block the installer. Click "More info" then "Run anyway".
