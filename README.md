### What is OverBind?
A utility that allows binding keyboard buttons to virtual Xbox360 controller joystick outputs. Makes use of [ViGEmBus](https://github.com/nefarius/ViGEmBus/) and [ViGEmClient](https://github.com/nefarius/ViGEmClient).

Built specifically with Hollow Knight speedruns in mind.

### How to Install
First, you must install the [ViGEmBus driver](https://github.com/nefarius/ViGEmBus/releases). This is necessary for the controller emulation to function.

Then install the OverBind executable from the [Releases page](https://github.com/cjonas1999/OverBind/releases).

### How to Configure
Included with the executable will be `OverBind_conf.txt`, which is required in the same location as the executable for the program to run.

Each row in this file corresponds to these controller inputs:
```
[Left Analog left]
[Left Analog right]
[Right analog up]
```
To customize these binds, you can find the list of [Virtual-Key Codes](https://learn.microsoft.com/en-us/windows/win32/inputdev/virtual-key-codes) here.

For reference the default binds are `Q,E,X`.

If you don't want something bound, you can just put a `0` in that row.
