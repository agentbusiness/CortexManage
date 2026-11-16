# cortexmanage

```sh
npx cortexmanage
```

Starts [CortexManage](https://github.com/agentbusiness/CortexManage), which manages the development of the Cortex apps. The first
run downloads the app for this machine (Apple Silicon Macs) from the official
releases, checks it against the checksum in the release catalog and unpacks it into `~/.cortexmanage/app/`.
Later runs start the installed app at once; the app updates its frame and plugins itself. It also
adds CortexManage to your applications (`/Applications`, or `~/Applications` when that is not
writable), which starts the newest installed version, so you
can open it without a terminal. The app
runs on its own: you can close the terminal (its start output goes to `~/.cortexmanage/logs/start.log`);
running the command again while it runs only says so.

Arguments go to the app: `npx cortexmanage --headless`, `npx cortexmanage --safe`.
