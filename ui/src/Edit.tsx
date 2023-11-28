import { invoke } from "@tauri-apps/api";
import { useEffect, useState } from "react";
import { MOD_KEYS, WINDOWS_ECMA_KEYMAP } from "./constants";

interface Keybind {
  id: number;
  name: string;
  keyName: string;
  keyCode: number;
}

const DEFAULT_BINDS: Keybind[] = [
  { id: 0, name: "Left Stick Left", keyName: "q", keyCode: 0x51 },
  { id: 1, name: "Left Stick Right", keyName: "e", keyCode: 0x45 },
  { id: 2, name: "Right Stick Up", keyName: "x", keyCode: 0x58 },
];

function KeybindSettings({
  onCancel,
  onSave,
  onErr,
}: {
  onCancel: () => void;
  onSave: () => void;
  onErr: (error: string) => void;
}) {
  // Function to handle save (to be implemented)
  const handleSave = () => {
    invoke("save_config", {
      codes: binds.map((bind) => bind.keyCode),
    })
      .then(() => onSave())
      .catch((err) => onErr(err));
  };

  const getKeybinds = () => {
    invoke("read_config")
      .then((response) => {
        const configBinds = response as number[];
        setBinds(
          binds.map((oldBind, i) => ({
            ...oldBind,
            keyCode: configBinds[i],
            keyName: Object.keys(WINDOWS_ECMA_KEYMAP).find(
              (key) => WINDOWS_ECMA_KEYMAP[key] === configBinds[i],
            )!,
          })),
        );
      })
      .catch((err) => onErr(err));
  };

  const [binds, setBinds] = useState(DEFAULT_BINDS);
  useEffect(getKeybinds, []);

  const [activeKeybindId, setActiveKeybindId] = useState<undefined | number>(
    undefined,
  );
  const [activeMods, setActiveMods] = useState(new Set());

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      let name = event.code;
      let winKeyCode = WINDOWS_ECMA_KEYMAP[name];
      if (!winKeyCode) {
        name = event.key;
        winKeyCode = WINDOWS_ECMA_KEYMAP[name];
      }
      if (winKeyCode) {
        if (MOD_KEYS.has(winKeyCode)) {
          setActiveMods(new Set(activeMods.add(winKeyCode)));
        } else if (activeKeybindId !== undefined) {
          // Record the key along with active mod keys
          const newKeybinds = binds.map((bind) =>
            bind.id === activeKeybindId
              ? {
                  ...bind,
                  keyName: name,
                  keyCode: winKeyCode,
                }
              : bind,
          );
          setBinds(newKeybinds);
          setActiveKeybindId(undefined); // Reset active keybind ID
          setActiveMods(new Set()); // Reset active mods
          window.removeEventListener("keydown", handleKeyDown);
          window.removeEventListener("keyup", handleKeyUp);
        }
      }
    };

    const handleKeyUp = (event: KeyboardEvent) => {
      let winKeyCode = WINDOWS_ECMA_KEYMAP[event.code];
      if (!winKeyCode) {
        winKeyCode = WINDOWS_ECMA_KEYMAP[event.key];
      }
      if (winKeyCode && MOD_KEYS.has(winKeyCode)) {
        activeMods.delete(winKeyCode);
        setActiveMods(new Set(activeMods));
      }
    };

    if (activeKeybindId !== undefined) {
      window.addEventListener("keydown", handleKeyDown);
      window.addEventListener("keyup", handleKeyUp);
    }

    return () => {
      if (activeKeybindId !== undefined) {
        window.removeEventListener("keydown", handleKeyDown);
        window.removeEventListener("keyup", handleKeyUp);
      }
    };
  }, [activeKeybindId, binds, activeMods]);

  const handleChangeKey = (id: number) => {
    setActiveKeybindId(id);
  };

  const cancelChangeKey = () => {
    setActiveKeybindId(undefined);
  };

  return (
    <div className="p-4 text-white">
      <h1 className="mb-4 text-lg font-bold">Keybind Settings</h1>
      <table className="mb-4 w-full table-auto">
        <thead>
          <tr className="bg-indigo-950 bg-opacity-60">
            <th className="px-4 py-2">Name</th>
            <th className="px-4 py-2">Key Name</th>
            <th className="px-4 py-2">Key Code</th>
            <th className="px-4 py-2">Actions</th>
          </tr>
        </thead>
        <tbody>
          {binds.map((bind) => (
            <tr
              key={bind.id}
              className="border-b border-indigo-950 bg-indigo-800 bg-opacity-60"
            >
              <td className="px-4 py-2">{bind.name}</td>
              <td className="px-4 py-2">
                {bind.id === activeKeybindId ? "..." : bind.keyName}
              </td>
              <td className="px-4 py-2">
                {bind.id === activeKeybindId
                  ? "..."
                  : bind.keyCode.toString(16)}
              </td>
              <td className="px-4 py-2">
                {bind.id === activeKeybindId ? (
                  <button
                    onClick={cancelChangeKey}
                    className="rounded bg-red-300 px-4 py-2 font-bold text-black hover:bg-red-500"
                  >
                    Cancel
                  </button>
                ) : (
                  <button
                    onClick={() => handleChangeKey(bind.id)}
                    className="rounded bg-purple-700 px-4 py-2 font-bold text-white hover:bg-blue-700"
                  >
                    Change Key
                  </button>
                )}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="flex justify-end">
        <button
          onClick={handleSave}
          className="mr-2 rounded bg-green-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-green-700"
        >
          Save
        </button>
        <button
          onClick={onCancel}
          className="rounded bg-red-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-red-700"
        >
          Cancel
        </button>
      </div>
    </div>
  );
}

export default KeybindSettings;
