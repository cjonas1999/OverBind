import { invoke } from "@tauri-apps/api";
import { useEffect, useState } from "react";
import { MOD_KEYS, WINDOWS_ECMA_KEYMAP, CONTROLLER_INPUTS } from "./constants";

interface Keybind {
  id: number;
  name: string;
  keyName: string;
  keyCode: number;
}

interface ConfigBind {
  keycode: string;
  result_type: string;
  result_value: number;
}

const DEFAULT_BINDS: Keybind[] = [
  { id: 0, name: "LEFT STICK LEFT", keyName: "q", keyCode: 0x51 },
  { id: 1, name: "LEFT STICK RIGHT", keyName: "e", keyCode: 0x45 },
  { id: 2, name: "RIGHT STICK UP", keyName: "x", keyCode: 0x58 },
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
  const handleSave = () => {
    const configToSave = binds.map((bind) => ({
      keycode: bind.keyCode.toString(16),
      ...CONTROLLER_INPUTS[bind.name],
    }));

    console.log(JSON.stringify(configToSave));

    invoke("save_config", { configs: configToSave })
      .then(() => onSave())
      .catch((err) => onErr(err));
  };

  const getKeybinds = () => {
    invoke("read_config")
      .then((response) => {
        console.log(JSON.stringify(response));
        const configBinds = response as ConfigBind[];
        setBinds(
          configBinds.map((configBind, i) => {
            const keyCode = parseInt(configBind.keycode, 16);
            const keyName =
              Object.keys(WINDOWS_ECMA_KEYMAP).find(
                (key) => WINDOWS_ECMA_KEYMAP[key] === keyCode,
              ) || "-";
            const input = Object.entries(CONTROLLER_INPUTS).find(
              ([_, value]) =>
                value.result_type === configBind.result_type &&
                value.result_value === configBind.result_value,
            );
            const name = input ? input[0] : "Unknown";
            return {
              id: i,
              name,
              keyName,
              keyCode,
            };
          }),
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
      console.log(`Detected key ${name}`);
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
    console.log(`Listening for keybind ${id}`);
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
              <td className="px-4 py-2">
                <select
                  name="name"
                  id="name"
                  className="bg-transparent"
                  value={bind.name}
                  onChange={(e) => {
                    const newKeybinds = binds.map((b) =>
                      b.id === bind.id
                        ? {
                            ...b,
                            name: e.target.value,
                          }
                        : b,
                    );
                    setBinds(newKeybinds);
                  }}
                >
                  {Object.keys(CONTROLLER_INPUTS).map((name) => (
                    <option
                      key={name}
                      value={name}
                      className="bg-indigo-800 bg-opacity-60 hover:bg-indigo-700"
                    >
                      {name}
                    </option>
                  ))}
                </select>
              </td>
              <td className="px-4 py-2">
                {bind.id === activeKeybindId ? "..." : bind.keyName}
              </td>
              <td className="px-4 py-2">
                {bind.id === activeKeybindId
                  ? "..."
                  : bind.keyCode.toString(16)}
              </td>
              <td className="flex justify-center gap-2.5 px-4 py-2">
                {bind.id === activeKeybindId ? (
                  <>
                    <button
                      onClick={cancelChangeKey}
                      className="rounded bg-rose-800 px-4 py-2 font-bold text-white hover:bg-rose-500"
                    >
                      Cancel
                    </button>
                    <button
                      onClick={() => {
                        setBinds(
                          binds.map((bind) =>
                            bind.id === activeKeybindId
                              ? {
                                  ...bind,
                                  keyName: "-",
                                  keyCode: 0,
                                }
                              : bind,
                          ),
                        );
                        setActiveKeybindId(undefined);
                      }}
                      className="rounded bg-orange-800 px-4 py-2 font-bold text-white hover:bg-orange-500"
                    >
                      Unbind
                    </button>
                  </>
                ) : (
                  <>
                    <button
                      onClick={() => handleChangeKey(bind.id)}
                      className="rounded bg-purple-700 px-4 py-2 font-bold text-white hover:bg-purple-500"
                    >
                      Rebind
                    </button>
                    <button
                      onClick={() =>
                        setBinds(binds.filter((b) => b.id !== bind.id))
                      }
                      className="rounded bg-rose-700 px-4 py-2 font-bold text-white hover:bg-rose-500"
                    >
                      Delete
                    </button>
                  </>
                )}
              </td>
            </tr>
          ))}
          <tr>
            <td
              colSpan={4}
              className="cursor-pointer border-b border-indigo-950 bg-slate-800 bg-opacity-60 hover:bg-slate-500"
              onClick={() => {
                setBinds([
                  ...binds,
                  {
                    id: binds.length,
                    name: "Unknown",
                    keyName: "-",
                    keyCode: 0,
                  },
                ]);
              }}
            >
              +
            </td>
          </tr>
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
