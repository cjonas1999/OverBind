import { invoke } from "@tauri-apps/api";
import { useEffect, useState } from "react";
import { MOD_KEYS, WINDOWS_ECMA_KEYMAP, CONTROLLER_INPUTS } from "../constants";
import Dropdown from "./Dropdown";

type BindType = "controller" | "keyboard" | undefined;

interface Keybind {
  id: number;
  type: BindType;
  name: string;
  keyName: string;
  keyCode: number;
  socd?: boolean;
}

interface ConfigBind {
  keycode: string;
  result_type: string;
  result_value: number;
}

const DEFAULT_BINDS: Keybind[] = [
  {
    id: 0,
    type: "controller",
    name: "LEFT STICK LEFT",
    keyName: "q",
    keyCode: 0x51,
  },
  {
    id: 1,
    type: "controller",
    name: "LEFT STICK RIGHT",
    keyName: "e",
    keyCode: 0x45,
  },
  {
    id: 2,
    type: "controller",
    name: "RIGHT STICK UP",
    keyName: "x",
    keyCode: 0x58,
  },
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
      ...(bind.type === "controller"
        ? CONTROLLER_INPUTS[bind.name]
        : {
            result_type: bind.socd ? "socd" : "keyboard",
            result_value: WINDOWS_ECMA_KEYMAP[bind.name],
          }),
    }));

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
            let type: BindType = "controller";
            let input: [string, any] | undefined = Object.entries(
              CONTROLLER_INPUTS,
            ).find(
              ([_, value]) =>
                value.result_type === configBind.result_type &&
                value.result_value === configBind.result_value,
            );
            if (!input) {
              type = "keyboard";
              input = Object.entries(WINDOWS_ECMA_KEYMAP).find(
                ([_, value]) => value === configBind.result_value,
              );
            }
            const name = input ? input[0] : "";
            return {
              id: i,
              type,
              name,
              keyName,
              keyCode,
              socd: configBind.result_type === "socd",
            };
          }),
        );
      })
      .catch((err) => onErr(err));
  };

  const setSocdLinkedBinds = (bindIdA: number, bindIdB: number) => {
    const bindA = binds.find((b) => b.id === bindIdA)!;
    const bindB = binds.find((b) => b.id === bindIdB)!;
    setBinds(
      binds.map((b) => {
        if (b.id === bindIdA) {
          return {
            ...b,
            keyName: bindB.name,
            keyCode: WINDOWS_ECMA_KEYMAP[bindB.name],
            socd: true,
          };
        } else if (b.id === bindIdB) {
          return {
            ...b,
            keyName: bindA.name,
            keyCode: WINDOWS_ECMA_KEYMAP[bindA.name],
            socd: true,
          };
        } else {
          return b;
        }
      }),
    );
  };

  const [binds, setBinds] = useState(DEFAULT_BINDS);
  useEffect(getKeybinds, []);

  const [activeKeybindId, setActiveKeybindId] = useState<undefined | number>(
    undefined,
  );
  const [activeMods, setActiveMods] = useState(new Set());

  const [linkedBinds, setLinkedBinds] = useState<number[] | undefined>(
    undefined,
  );

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
        // if (MOD_KEYS.has(winKeyCode)) {
        //   setActiveMods(new Set(activeMods.add(winKeyCode)));
        // } else
        if (activeKeybindId !== undefined) {
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

  const handleChangeKey = (
    id: number,
    event: React.MouseEvent<HTMLButtonElement>,
  ) => {
    console.log(`Listening for keybind ${id}`);
    (event.target as HTMLButtonElement).blur();
    setActiveKeybindId(id);
  };

  const cancelChangeKey = () => {
    setActiveKeybindId(undefined);
  };

  return (
    <div className="p-4 text-white">
      <div
        className="fixed inset-0 z-10 flex items-center justify-center"
        style={{ display: linkedBinds ? "flex" : "none" }}
      >
        <div className="absolute inset-0 bg-black opacity-50"></div>
        <div className="relative mx-auto max-w-lg rounded-lg bg-gray-900 p-4 text-center shadow-lg">
          <h1 className="mb-4 text-lg font-bold">Warning</h1>
          <p className="mb-4">
            You are linking two cardinal direction binds for simultaneous
            opposite direction override (SOCD cleaning). This will override the
            existing keys for both linked binds.
          </p>
          <div className="flex justify-center">
            <button
              onClick={() => {
                setSocdLinkedBinds(linkedBinds![0], linkedBinds![1]);
                cancelChangeKey();
                setLinkedBinds(undefined);
              }}
              className="mr-2 rounded bg-green-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-green-800"
            >
              Confirm
            </button>
            <button
              onClick={() => {
                cancelChangeKey();
                setLinkedBinds(undefined);
              }}
              className="rounded bg-red-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-red-700"
            >
              Cancel
            </button>
          </div>
        </div>
      </div>
      <h1 className="mb-4 text-lg font-bold">Keybind Settings</h1>
      <table className="mb-4 w-full table-auto">
        <thead>
          <tr className="bg-indigo-950 bg-opacity-60">
            <th className="px-4 py-2"></th>
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
              <td className="py-2 pl-4">
                {bind.socd ? (
                  <svg
                    id="link"
                    data-name="link"
                    xmlns="http://www.w3.org/2000/svg"
                    viewBox="0 0 122.88 122.88"
                    width="15px"
                    height="15px"
                    fill="white"
                  >
                    <path d="M60.54,34.07A7.65,7.65,0,0,1,49.72,23.25l13-12.95a35.38,35.38,0,0,1,49.91,0l.07.08a35.37,35.37,0,0,1-.07,49.83l-13,12.95A7.65,7.65,0,0,1,88.81,62.34l13-13a20.08,20.08,0,0,0,0-28.23l-.11-.11a20.08,20.08,0,0,0-28.2.07l-12.95,13Zm14,3.16A7.65,7.65,0,0,1,85.31,48.05L48.05,85.31A7.65,7.65,0,0,1,37.23,74.5L74.5,37.23ZM62.1,89.05A7.65,7.65,0,0,1,72.91,99.87l-12.7,12.71a35.37,35.37,0,0,1-49.76.14l-.28-.27a35.38,35.38,0,0,1,.13-49.78L23,50A7.65,7.65,0,1,1,33.83,60.78L21.12,73.49a20.09,20.09,0,0,0,0,28.25l0,0a20.07,20.07,0,0,0,28.27,0L62.1,89.05Z" />
                  </svg>
                ) : (
                  ""
                )}
              </td>
              <td className="justify-left flex gap-2.5 px-4 py-2">
                <Dropdown
                  options={["controller", "keyboard"]}
                  onChange={(option) => {
                    const newKeybinds = binds.map((b) =>
                      b.id === bind.id
                        ? {
                            ...b,
                            type: option as BindType,
                            name: "",
                          }
                        : b,
                    );
                    setBinds(newKeybinds);
                  }}
                >
                  {bind.type === "controller" ? (
                    <svg
                      version="Controller"
                      id="Controller"
                      xmlns="http://www.w3.org/2000/svg"
                      x="0px"
                      y="0px"
                      width={37}
                      viewBox="0 0 122.88 79.92"
                      fill="#fff"
                    >
                      <g>
                        <path d="M23.35,72.21c4.04-4.11,8.82-8.28,12.37-13.68h51.43c3.56,5.39,8.34,9.57,12.37,13.68 c30.95,31.52,28.87-42.32,7-64.5h-1.68C102.09,3.11,96.72,0,90.55,0c-6.17,0-11.53,3.11-14.28,7.71H46.61 C43.86,3.11,38.49,0,32.32,0c-6.17,0-11.53,3.11-14.29,7.71h-1.68C-5.52,29.89-7.6,103.72,23.35,72.21L23.35,72.21z M29.85,12.84 h11.11v8.85l8.85,0V32.8h-8.85v8.85H29.85V32.8H21V21.69h8.85L29.85,12.84L29.85,12.84L29.85,12.84z M83.16,36.9 c2.69,0,4.87,2.18,4.87,4.87c0,2.69-2.18,4.88-4.87,4.88s-4.87-2.18-4.87-4.88C78.29,39.08,80.47,36.9,83.16,36.9L83.16,36.9z M85.82,15.21c3.9,0,7.06,3.16,7.06,7.05c0,3.9-3.16,7.05-7.06,7.05c-3.9,0-7.05-3.16-7.05-7.05 C78.77,18.37,81.92,15.21,85.82,15.21L85.82,15.21z M104.04,26.11c2.69,0,4.87,2.18,4.87,4.87c0,2.69-2.18,4.87-4.87,4.87 c-2.69,0-4.88-2.18-4.88-4.87C99.16,28.29,101.35,26.11,104.04,26.11L104.04,26.11z" />
                      </g>
                    </svg>
                  ) : bind.type === "keyboard" ? (
                    <svg
                      version="Keyboard"
                      id="Keyboard"
                      xmlns="http://www.w3.org/2000/svg"
                      x="0px"
                      y="0px"
                      width={49}
                      viewBox="0 0 122.88 59.48"
                      fill="#fff"
                    >
                      <g>
                        <path d="M113.82,0c2.49,0,4.76,1.02,6.4,2.66c1.64,1.64,2.66,3.91,2.66,6.4v41.35c0,2.49-1.02,4.76-2.66,6.4 c-1.64,1.64-3.91,2.66-6.4,2.66H9.06c-2.49,0-4.76-1.02-6.4-2.66C1.02,55.18,0,52.91,0,50.42V9.06c0-2.49,1.02-4.76,2.66-6.4 C4.3,1.02,6.57,0,9.06,0C69.6,0,96.63,0,113.82,0L113.82,0z M92.24,3.84H9.06c-1.44,0-2.74,0.59-3.69,1.54 C4.42,6.32,3.84,7.63,3.84,9.06v41.35c0,1.44,0.59,2.74,1.54,3.69c0.95,0.95,2.25,1.54,3.69,1.54h104.75 c1.44,0,2.74-0.59,3.69-1.54c0.95-0.95,1.54-2.25,1.54-3.69V9.06c0-1.44-0.59-2.74-1.54-3.69c-0.95-0.95-2.25-1.54-3.69-1.54 h-13.24C98.26,3.84,94.56,3.84,92.24,3.84L92.24,3.84z M12.26,9.73h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54 c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C10.34,10.59,11.2,9.73,12.26,9.73L12.26,9.73z M27.61,9.73 h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54 C25.69,10.59,26.55,9.73,27.61,9.73L27.61,9.73z M42.97,9.73h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92 h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C41.05,10.59,41.91,9.73,42.97,9.73L42.97,9.73z M58.32,9.73h7.75 c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54 C56.41,10.59,57.26,9.73,58.32,9.73L58.32,9.73z M73.68,9.73h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92 h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C71.76,10.59,72.62,9.73,73.68,9.73L73.68,9.73z M89.04,9.73h7.75 c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54 C87.12,10.59,87.98,9.73,89.04,9.73L89.04,9.73z M104.39,9.73h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92 h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C102.47,10.59,103.33,9.73,104.39,9.73L104.39,9.73z M104.39,23.85h7.75 c1.06,0,1.92,0.87,1.92,1.94v20.68c0,1.07-0.86,1.94-1.92,1.94h-7.75c-1.06,0-1.92-0.87-1.92-1.94V25.79 C102.47,24.72,103.33,23.85,104.39,23.85L104.39,23.85z M12.26,24.02h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54 c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C10.34,24.88,11.2,24.02,12.26,24.02L12.26,24.02z M27.61,24.02h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54 C25.69,24.88,26.55,24.02,27.61,24.02L27.61,24.02z M42.97,24.02h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92 h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C41.05,24.88,41.91,24.02,42.97,24.02L42.97,24.02z M58.32,24.02h7.75 c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54 C56.41,24.88,57.26,24.02,58.32,24.02L58.32,24.02z M73.68,24.02h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92 h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C71.76,24.88,72.62,24.02,73.68,24.02L73.68,24.02z M89.04,24.02h7.75 c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54 C87.12,24.88,87.98,24.02,89.04,24.02L89.04,24.02z M12.26,38.16h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92 h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C10.34,39.02,11.2,38.16,12.26,38.16L12.26,38.16z M27.61,38.16h38.47 c1.05,0,1.9,0.86,1.9,1.92v6.54c0,1.06-0.85,1.92-1.9,1.92H27.61c-1.05,0-1.9-0.86-1.9-1.92v-6.54 C25.71,39.02,26.56,38.16,27.61,38.16L27.61,38.16z M73.68,38.16h7.75c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92 h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54C71.76,39.02,72.62,38.16,73.68,38.16L73.68,38.16z M89.04,38.16h7.75 c1.06,0,1.92,0.86,1.92,1.92v6.54c0,1.06-0.86,1.92-1.92,1.92h-7.75c-1.06,0-1.92-0.86-1.92-1.92v-6.54 C87.12,39.02,87.98,38.16,89.04,38.16L89.04,38.16z" />
                      </g>
                    </svg>
                  ) : (
                    ""
                  )}
                </Dropdown>
                {bind.type === "controller" ? (
                  <Dropdown
                    options={Object.keys(CONTROLLER_INPUTS)}
                    onChange={(option) => {
                      const newKeybinds = binds.map((b) =>
                        b.id === bind.id
                          ? {
                              ...b,
                              name: option,
                              type: "controller" as BindType,
                            }
                          : b,
                      );
                      setBinds(newKeybinds);
                    }}
                  >
                    {bind.name}
                  </Dropdown>
                ) : (
                  <Dropdown
                    options={Object.keys(WINDOWS_ECMA_KEYMAP)}
                    onChange={(option) => {
                      const newKeybinds = binds.map((b) =>
                        b.id === bind.id
                          ? {
                              ...b,
                              name: option,
                              type: "keyboard" as BindType,
                            }
                          : b,
                      );
                      setBinds(newKeybinds);
                    }}
                  >
                    {bind.name}
                  </Dropdown>
                )}
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
                ) : activeKeybindId !== undefined ? (
                  <button
                    onClick={() => {
                      const activeBind = binds.find(
                        (b) => b.id === activeKeybindId,
                      )!;
                      if (activeBind.keyCode !== 0 || bind.keyCode !== 0) {
                        setLinkedBinds([activeKeybindId, bind.id]);
                      } else {
                        setSocdLinkedBinds(activeKeybindId, bind.id);
                      }
                      cancelChangeKey();
                    }}
                    className={`rounded bg-purple-700 px-4 py-2 font-bold text-white hover:bg-purple-500 ${
                      bind.type === "keyboard" &&
                      binds.find((b) => b.id === activeKeybindId)!.type ===
                        "keyboard"
                        ? ""
                        : "hidden"
                    }`}
                  >
                    <svg
                      id="Link"
                      data-name="Link"
                      xmlns="http://www.w3.org/2000/svg"
                      viewBox="0 0 122.88 122.88"
                      width="20px"
                      height="20px"
                      fill="white"
                    >
                      <path d="M60.54,34.07A7.65,7.65,0,0,1,49.72,23.25l13-12.95a35.38,35.38,0,0,1,49.91,0l.07.08a35.37,35.37,0,0,1-.07,49.83l-13,12.95A7.65,7.65,0,0,1,88.81,62.34l13-13a20.08,20.08,0,0,0,0-28.23l-.11-.11a20.08,20.08,0,0,0-28.2.07l-12.95,13Zm14,3.16A7.65,7.65,0,0,1,85.31,48.05L48.05,85.31A7.65,7.65,0,0,1,37.23,74.5L74.5,37.23ZM62.1,89.05A7.65,7.65,0,0,1,72.91,99.87l-12.7,12.71a35.37,35.37,0,0,1-49.76.14l-.28-.27a35.38,35.38,0,0,1,.13-49.78L23,50A7.65,7.65,0,1,1,33.83,60.78L21.12,73.49a20.09,20.09,0,0,0,0,28.25l0,0a20.07,20.07,0,0,0,28.27,0L62.1,89.05Z" />
                    </svg>
                  </button>
                ) : (
                  <>
                    <button
                      onClick={(event) => handleChangeKey(bind.id, event)}
                      className="rounded bg-purple-700 px-4 py-2 font-bold text-white hover:bg-purple-500"
                    >
                      <svg
                        version="Edit"
                        id="Edit"
                        xmlns="http://www.w3.org/2000/svg"
                        x="0px"
                        y="0px"
                        width="20px"
                        height="20px"
                        viewBox="0 0 122.88 121.96"
                        fill="white"
                      >
                        <g>
                          <path d="M107.73,1.31c-0.96-0.89-2.06-1.37-3.29-1.3c-1.23,0-2.33,0.48-3.22,1.44l-7.27,7.54l20.36,19.67l7.33-7.68 c0.89-0.89,1.23-2.06,1.23-3.29c0-1.23-0.48-2.4-1.37-3.22L107.73,1.31L107.73,1.31L107.73,1.31z M8.35,5.09h50.2v13.04H14.58 c-0.42,0-0.81,0.18-1.09,0.46c-0.28,0.28-0.46,0.67-0.46,1.09v87.71c0,0.42,0.18,0.81,0.46,1.09c0.28,0.28,0.67,0.46,1.09,0.46 h87.71c0.42,0,0.81-0.18,1.09-0.46c0.28-0.28,0.46-0.67,0.46-1.09V65.1h13.04v48.51c0,2.31-0.95,4.38-2.46,5.89 c-1.51,1.51-3.61,2.46-5.89,2.46H8.35c-2.32,0-4.38-0.95-5.89-2.46C0.95,118,0,115.89,0,113.61V13.44c0-2.32,0.95-4.38,2.46-5.89 C3.96,6.04,6.07,5.09,8.35,5.09L8.35,5.09z M69.62,75.07c-2.67,0.89-5.42,1.71-8.09,2.61c-2.67,0.89-5.35,1.78-8.09,2.67 c-6.38,2.06-9.87,3.22-10.63,3.43c-0.75,0.21-0.27-2.74,1.3-8.91l5.07-19.4l0.42-0.43l20.02,20.02L69.62,75.07L69.62,75.07 L69.62,75.07z M57.01,47.34L88.44,14.7l20.36,19.6L77.02,67.35L57.01,47.34L57.01,47.34z" />
                        </g>
                      </svg>
                    </button>
                    <button
                      onClick={() =>
                        setBinds(binds.filter((b) => b.id !== bind.id))
                      }
                      className="rounded bg-rose-700 px-4 py-2 font-bold text-white hover:bg-rose-500"
                    >
                      <svg
                        version="Delete"
                        id="Delete"
                        xmlns="http://www.w3.org/2000/svg"
                        x="0px"
                        y="0px"
                        width="20px"
                        height="20px"
                        viewBox="0 0 121.31 122.876"
                        fill="white"
                      >
                        <g>
                          <path
                            fill-rule="evenodd"
                            clip-rule="evenodd"
                            d="M90.914,5.296c6.927-7.034,18.188-7.065,25.154-0.068 c6.961,6.995,6.991,18.369,0.068,25.397L85.743,61.452l30.425,30.855c6.866,6.978,6.773,18.28-0.208,25.247 c-6.983,6.964-18.21,6.946-25.074-0.031L60.669,86.881L30.395,117.58c-6.927,7.034-18.188,7.065-25.154,0.068 c-6.961-6.995-6.992-18.369-0.068-25.397l30.393-30.827L5.142,30.568c-6.867-6.978-6.773-18.28,0.208-25.247 c6.983-6.963,18.21-6.946,25.074,0.031l30.217,30.643L90.914,5.296L90.914,5.296z"
                          />
                        </g>
                      </svg>
                    </button>
                  </>
                )}
              </td>
            </tr>
          ))}
          <tr>
            <td
              colSpan={5}
              className="cursor-pointer border-b border-indigo-950 bg-slate-800 bg-opacity-60 hover:bg-slate-500"
              onClick={() => {
                setBinds([
                  ...binds,
                  {
                    id: binds.length,
                    name: "",
                    type: undefined,
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
