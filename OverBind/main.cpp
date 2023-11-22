// most of this is just copied from the example at https://github.com/nefarius/ViGEmClient
#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <ViGEm/Client.h>
#include <iostream>
#include <sstream>
#include <fstream>
#include <string>
#pragma comment(lib, "setupapi.lib")

#define STICK_NEUTRAL 0
#define STICK_LEFT -29000
#define STICK_RIGHT 29000
#define STICK_UP 29000
#define STICK_DOWN -29000

#define CONFIG_FILE_NAME "OverBind_conf.txt"
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
        for (int i = 0; i < 3; i++) {
            if (kbdStruct.vkCode == KEYBINDS[i]) {
                key_held[i] = 0;
            }
        }
    }

    if (wParam == WM_KEYDOWN) {
        for (int i = 0; i < 3; i++) {
            if (kbdStruct.vkCode == KEYBINDS[i]) {
                key_held[i] = 1;
            }
        }
    }


    SHORT left_stick_X = STICK_NEUTRAL;
    SHORT left_stick_Y = STICK_NEUTRAL;
    SHORT right_stick_X = STICK_NEUTRAL;
    SHORT right_stick_Y = STICK_NEUTRAL;

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

    XUSB_REPORT inputs = {
       0,
       0,
       0,
       left_stick_X,
       left_stick_Y,
       right_stick_X,
       right_stick_Y
    };

    vigem_target_x360_update(client, pad, inputs);

    // call the next hook in the hook chain. This is nessecary or your hook chain will break and the hook stops
    return CallNextHookEx(keyboard_hook, nCode, wParam, lParam);
}





int main() {
//Create low level hook for keyboard
    keyboard_hook = SetWindowsHookEx(WH_KEYBOARD_LL, &HookProc, NULL, 0);




 //Create virtual controller
    client = vigem_alloc();

    if (client == nullptr) {
        std::cerr << "Not enough memory to launch virtual controller client." << std::endl;
        MessageBoxA(NULL, "Not enough memory to launch virtual controller client.", "Error!", MB_ICONERROR | MB_OK);
        return -1;
    }

    const auto retval = vigem_connect(client);

    if (!VIGEM_SUCCESS(retval)) {
        
        std::stringstream ss;
        ss << "ViGEm Bus connection failed with error code: 0x" << std::hex << retval;
        ss << "\nYou may need to download the virtual gamepad driver here: https://github.com/nefarius/ViGEmBus/releases";
        std::string errorText = ss.str();
        std::cerr << errorText << std::endl;
        MessageBoxA(NULL, errorText.c_str(), "Error!", MB_ICONERROR | MB_OK);
        return -1;
    }

    // Allocate handle to identify new pad
    pad = vigem_target_x360_alloc();

    // Add client to the bus, this equals a plug-in event
    const auto pir = vigem_target_add(client, pad);

    // Error handling
    if (!VIGEM_SUCCESS(pir)) {
        std::stringstream ss;
        ss << "Target plugin failed with error code: 0x" << std::hex << pir;
        auto errorText = ss.str();
        std::cerr << errorText << std::endl;
        MessageBoxA(NULL, errorText.c_str(), "Error!", MB_ICONERROR | MB_OK);
        return -1;
    }



 // read config file
    std::ifstream config_file (CONFIG_FILE_NAME);

    if (!config_file.is_open()) {
        std::cerr << "Config file could not be found" << std::endl;
        MessageBoxA(NULL, "Config file could not be found", "Error!", MB_ICONERROR | MB_OK);
        return -1;
    }

    for (int i = 0; i < 3; i++) {
        std::string str;
        std::getline(config_file, str);
        KEYBINDS[i] = std::stol(str, nullptr, 16);
    }
    config_file.close();


    std::cout << "OverBind is running" << std::endl;
    MSG msg;
    while (GetMessageW(&msg, NULL, 0, 0)) {
        TranslateMessage(&msg);
        DispatchMessage(&msg);
    }

//cleanup
    vigem_target_remove(client, pad);
    vigem_target_free(pad);
}