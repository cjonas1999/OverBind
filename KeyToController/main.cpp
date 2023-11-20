// most of this is just copied from the example at https://github.com/nefarius/ViGEmClient
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <ViGEm/Client.h>
#include <iostream>
#include <sstream>
#include <fstream>
#include <string>
#pragma comment(lib, "setupapi.lib")

#define STICK_NEUTRAL 128
#define STICK_LEFT 6
#define STICK_RIGHT 250
#define STICK_UP 6
#define STICK_DOWN 250

#define CONFIG_FILE_NAME "ktc_conf.txt"
#define KEYBIND_LEFT_STICK_LEFT 0
#define KEYBIND_LEFT_STICK_RIGHT 1
#define KEYBIND_RIGHT_STICK_UP 2
int KEYBINDS[3];


HHOOK keyboard_hook;

PVIGEM_CLIENT client;
PVIGEM_TARGET pad;

int key_held[3];

LRESULT __stdcall HookProc(int nCode, WPARAM wParam, LPARAM lParam) {

    if (nCode != HC_ACTION) {
        return CallNextHookEx(NULL, nCode, wParam, lParam);
    }

    KBDLLHOOKSTRUCT kbdStruct = *((KBDLLHOOKSTRUCT*)lParam);

    if (wParam == WM_KEYUP) {
        if (kbdStruct.vkCode == KEYBINDS[KEYBIND_LEFT_STICK_LEFT]) {
            key_held[KEYBIND_LEFT_STICK_LEFT] = 0;
        }

        if (kbdStruct.vkCode == KEYBINDS[KEYBIND_LEFT_STICK_RIGHT]) {
            key_held[KEYBIND_LEFT_STICK_RIGHT] = 0;
        }

        if (kbdStruct.vkCode == KEYBINDS[KEYBIND_RIGHT_STICK_UP]) {
            key_held[KEYBIND_RIGHT_STICK_UP] = 0;
        }
    }

    if (wParam == WM_KEYDOWN) {

        if (kbdStruct.vkCode == KEYBINDS[KEYBIND_LEFT_STICK_LEFT]) {
            key_held[KEYBIND_LEFT_STICK_LEFT] = 1;
        }

        if (kbdStruct.vkCode == KEYBINDS[KEYBIND_LEFT_STICK_RIGHT]) {
            key_held[KEYBIND_LEFT_STICK_RIGHT] = 1;
        }

        if (kbdStruct.vkCode == KEYBINDS[KEYBIND_RIGHT_STICK_UP]) {
            key_held[KEYBIND_RIGHT_STICK_UP] = 1;
        }
    }


    BYTE left_stick_X = STICK_NEUTRAL;
    BYTE left_stick_Y = STICK_NEUTRAL;
    BYTE right_stick_X = STICK_NEUTRAL;
    BYTE right_stick_Y = STICK_NEUTRAL;

    if (key_held[KEYBIND_LEFT_STICK_RIGHT]) {
        left_stick_X = STICK_RIGHT;
    }
    else if (key_held[KEYBIND_LEFT_STICK_LEFT]) {
        left_stick_X = STICK_LEFT;
    }
    else {
        left_stick_X = STICK_NEUTRAL;
    }

    if (key_held[KEYBIND_RIGHT_STICK_UP]) {
        right_stick_Y = STICK_UP;
    }
    else {
        right_stick_Y = STICK_NEUTRAL;
    }

    DS4_REPORT inputs = {
       left_stick_X,
       left_stick_Y,
       right_stick_X,
       right_stick_Y,
       8,
       0,
       0,
       0
    };

    vigem_target_ds4_update(client, pad, inputs);

    // call the next hook in the hook chain. This is nessecary or your hook chain will break and the hook stops
    return CallNextHookEx(keyboard_hook, nCode, wParam, lParam);
}





int main() {
//Create low level hook for keyboard
    keyboard_hook = SetWindowsHookEx(WH_KEYBOARD_LL, &HookProc, NULL, 0);




 //Create virtual controller
    client = vigem_alloc();

    if (client == nullptr) {
        std::cerr << "Uh, not enough memory to do that?!" << std::endl;
        return -1;
    }

    const auto retval = vigem_connect(client);

    if (!VIGEM_SUCCESS(retval)) {
        std::cerr << "ViGEm Bus connection failed with error code: 0x" << std::hex << retval << std::endl;
        return -1;
    }

    // Allocate handle to identify new pad
    pad = vigem_target_ds4_alloc();

    // Add client to the bus, this equals a plug-in event
    const auto pir = vigem_target_add(client, pad);

    // Error handling
    if (!VIGEM_SUCCESS(pir)) {
        std::cerr << "Target plugin failed with error code: 0x" << std::hex << pir << std::endl;
        return -1;
    }



 // read config file
    std::ifstream config_file (CONFIG_FILE_NAME);

    if (!config_file.is_open()) {
        std::cerr << "Config file could not be found" << std::endl;
        return 1;
    }

    for (int i = 0; i < 3; i++) {
        std::string str;
        std::getline(config_file, str);
        KEYBINDS[i] = std::stol(str, nullptr, 16);
    }
    config_file.close();

    MSG msg;
    while (GetMessageW(&msg, NULL, 0, 0)) {
    }

//cleanup
    vigem_target_remove(client, pad);
    vigem_target_free(pad);
}