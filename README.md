## What is OverBind?
A utility that allows binding keyboard buttons to virtual Xbox360 controller joystick outputs. Makes use of [ViGEmBus](https://github.com/nefarius/ViGEmBus/) and [ViGEmClient](https://github.com/CasualX/vigem-client). Keybinds are easily customizeable within the app's UI.

Built specifically with Hollow Knight speedruns in mind.

## How to Install

### Windows
First, you must install the [ViGEmBus driver](https://github.com/nefarius/ViGEmBus/releases). This is necessary for the controller emulation to function.

Then install and run the OverBind installer from the [Releases page](https://github.com/cjonas1999/OverBind/releases).

### Linux
The following are the recommended instructions for setting up the appropriate permissions to allow OverBind to access your input device.

1. Create file `/etc/udev/rules.d/99-uinput.rules`
2. Paste contents into file `KERNEL=="uinput", GROUP="input", MODE="0660"`. This grants permission to read and write to your input devices to anyone in the "input" group.
3. Run command `sudo usermod -aG input $(whoami)`. This adds the current user to the input group.
4. Create file `/etc/modules-load.d/uinput.conf`
5. Paste contents into file `uinput`
6. Restart computer
7. Set your device in the overbind settings in the "Input Devices" dropdown.

## How to Build
Overbind is written in Rust and uses the [Tauri](https://tauri.app/) framework. To build OverBind, you will need to install the following dependencies:
- [Rust](https://www.rust-lang.org/tools/install)
- [Node.js](https://nodejs.org/en/download/)

Launch dev mode with `npm run tauri dev` and build with `npm run tauri build`.
