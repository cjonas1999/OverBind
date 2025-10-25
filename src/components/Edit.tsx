import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { WINDOWS_ECMA_KEYMAP, CONTROLLER_INPUTS } from "../constants";
import Dropdown from "./Dropdown";
import { InputTypeIcon } from "./InputTypeIcon";

type BindType = "controller" | "keyboard" | "socd" | "mash_trigger" | undefined;

interface Keybind {
  id: number;
  type: BindType;
  output: string;
  input: string;
}

interface ConfigBind {
  keycode: string;
  result_type: string;
  result_value: number;
}

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
      keycode: WINDOWS_ECMA_KEYMAP[bind.input].toString(16),
      ...(bind.type === "controller"
        ? CONTROLLER_INPUTS[bind.output]
        : {
          result_type: bind.type,
          result_value: WINDOWS_ECMA_KEYMAP[bind.output],
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
        setBindsCount(configBinds.length);
        const newBinds = configBinds.map((configBind, i) => {
          const keyIn = parseInt(configBind.keycode, 16);
          const input =
            Object.entries(WINDOWS_ECMA_KEYMAP).find(
              ([_, value]) => value === keyIn,
            )?.[0] ?? "";
          let type: BindType = "controller";
          let output: string =
            Object.entries(CONTROLLER_INPUTS).find(
              ([_, value]) =>
                value.result_type === configBind.result_type &&
                value.result_value === configBind.result_value,
            )?.[0] ?? "";
          if (!output) {
            type = configBind.result_type as BindType;
            output =
              Object.entries(WINDOWS_ECMA_KEYMAP).find(
                ([_, value]) => value === configBind.result_value,
              )?.[0] ?? "";
          }
          return {
            id: i,
            type,
            input,
            output,
          };
        });
        const linkedBinds: number[][] = [];
        newBinds.forEach((bind) => {
          if (bind.type === "socd") {
            const otherBind = newBinds.find(
              (b) =>
                b.id !== bind.id &&
                b.type === "socd" &&
                b.input === bind.output,
            );
            if (otherBind && !linkedBinds.find((b) => b.includes(bind.id))) {
              linkedBinds.push([bind.id, otherBind.id]);
            }
          }
        });
        setBinds(newBinds);
        setLinkedBinds(linkedBinds);
      })
      .catch((err) => onErr(err));
  };

  const setSocdLinkedBinds = (
    newBinds: Keybind[],
    bindIdA: number,
    bindIdB: number,
    setByInput: boolean,
  ) => {
    const bindA = newBinds.find((b) => b.id === bindIdA)!;
    const bindB = newBinds.find((b) => b.id === bindIdB)!;
    setBinds(
      newBinds.map((b) => {
        if (b.id === bindIdA) {
          return {
            ...b,
            input: setByInput ? bindA.input : bindB.output,
            output: setByInput ? bindB.input : bindA.output,
          };
        } else if (b.id === bindIdB) {
          return {
            ...b,
            input: setByInput ? bindB.input : bindA.output,
            output: setByInput ? bindA.input : bindB.output,
          };
        } else {
          return b;
        }
      }),
    );
  };

  const [binds, setBinds] = useState<Keybind[]>([]);
  const [bindsCount, setBindsCount] = useState(0);
  useEffect(getKeybinds, []);

  const [activeKeybindId, setActiveKeybindId] = useState<
    undefined | [number, boolean]
  >(undefined);

  const [linkedBinds, setLinkedBinds] = useState<number[][]>([]);

  const [newBindDropdownOpen, setNewBindDropdownOpen] = useState({
    open: false,
    x: 0,
    y: 0,
  });

  const [infoModalOpen, setInfoModalOpen] = useState(false);

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      console.log("Keydown event", event);
      let name = event.code;
      console.log(`Detected key ${name}`);
      let winKeyCode = WINDOWS_ECMA_KEYMAP[name];
      if (!winKeyCode) {
        name = event.key;
        winKeyCode = WINDOWS_ECMA_KEYMAP[name];
      }
      if (winKeyCode) {
        if (activeKeybindId !== undefined) {
          const bind = binds.find((b) => b.id === activeKeybindId![0])!;
          // Record the key along with active mod keys
          const newKeybinds = binds.map((b) =>
            b.id === bind.id
              ? {
                ...b,
                input: activeKeybindId[1] ? name : b.input,
                output: activeKeybindId[1] && bind.type !== "mash_trigger" ? b.output : name,
              }
              : b,
          );
          setBinds(newKeybinds);

          if (bind.type === "socd") {
            const theseLinkedBinds = linkedBinds.find(
              (b) => b[0] === bind.id || b[1] === bind.id,
            );
            setSocdLinkedBinds(
              newKeybinds,
              theseLinkedBinds![0],
              theseLinkedBinds![1],
              activeKeybindId[1],
            );
          }
          setActiveKeybindId(undefined); // Reset active keybind ID
          console.log("Removed keydown listener");
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
    };

    if (activeKeybindId !== undefined) {
      console.log("Adding keydown listener");
      window.addEventListener("keydown", handleKeyDown);
      window.addEventListener("keyup", handleKeyUp);
    }

    return () => {
      if (activeKeybindId !== undefined) {
        console.log("Removed keydown listener");
        window.removeEventListener("keydown", handleKeyDown);
        window.removeEventListener("keyup", handleKeyUp);
      }
    };
  }, [activeKeybindId, binds]);

  const cancelChangeKey = (id: number, setByInput: boolean) => {
    if (
      activeKeybindId &&
      activeKeybindId?.[0] === id &&
      activeKeybindId?.[1] === setByInput
    ) {
      console.log("Cancelled keybind change");
      setActiveKeybindId(undefined);
    }
  };

  return (
    <>
    <div className="p-4 text-white">
      <span className="flex flex-row justify-center mb-4 gap-2">
        <h1 className="mb-0 text-lg font-bold">Edit Keybinds</h1>
        <div className="cursor-pointer" onClick={() => setInfoModalOpen(true)}>ℹ️</div>
      </span>
      <table className="mb-4 w-full table-auto">
        <thead>
          <tr className="bg-indigo-950 bg-opacity-60">
            <th className="px-4 py-2">Type</th>
            <th className="px-4 py-2">Output</th>
            <th className="px-4 py-2"></th>
            <th className="px-4 py-2">Input</th>
            <th className="px-0 py-2"></th>
          </tr>
        </thead>
        <tbody>
          {binds.map((bind) => (
            <tr
              key={bind.id}
              className="border-b border-indigo-950 bg-indigo-800 bg-opacity-60"
            >
              <td className="object-center">
                <div className="flex justify-center">
                  {bind.type ? (
                    <InputTypeIcon
                      type={bind.type}
                      badge={
                        bind.type === "socd"
                          ? (
                            linkedBinds.findIndex(
                              (b) => b[0] === bind.id || b[1] === bind.id,
                            ) + 1
                          ).toString()
                          : undefined
                      }
                    />
                  ) : (
                    ""
                  )}
                </div>
              </td>
              <td className="px-4 py-2">
                {bind.type !== "mash_trigger" && (<>
                  {bind.type === "controller" ? (
                    <Dropdown
                      options={Object.keys(CONTROLLER_INPUTS)}
                      onChange={(option) => {
                        const newKeybinds = binds.map((b) =>
                          b.id === bind.id
                            ? {
                              ...b,
                              output: option,
                              type: bind.type,
                            }
                            : b,
                        );
                        setBinds(newKeybinds);
                      }}
                    >
                      {bind.output}
                    </Dropdown>
                  ) : (
                    <Dropdown
                      options={Object.keys(WINDOWS_ECMA_KEYMAP)}
                      onOpen={() => {
                        setActiveKeybindId([bind.id, false]);
                      }}
                      onBlur={() => cancelChangeKey(bind.id, false)}
                      onChange={(option) => {
                        const newKeybinds = binds.map((b) =>
                          b.id === bind.id
                            ? {
                              ...b,
                              output: option,
                              type: bind.type,
                            }
                            : b,
                        );
                        setBinds(newKeybinds);
                        if (bind.type === "socd") {
                          const theseLinkedBinds = linkedBinds.find(
                            (b) => b[0] === bind.id || b[1] === bind.id,
                          );
                          setSocdLinkedBinds(
                            newKeybinds,
                            theseLinkedBinds![0],
                            theseLinkedBinds![1],
                            false,
                          );
                        }
                      }}
                      openAt={
                        bind.id === activeKeybindId?.[0] && !activeKeybindId?.[1]
                          ? undefined
                          : { open: false, x: -500, y: 0 }
                      }
                    >
                      {bind.output}
                    </Dropdown>
                  )}
                </>)}
              </td>
              <td className="px-4 py-2 text-3xl">
                {bind.type === "mash_trigger" ? "" : (bind.type === "socd" ? "↔" : "←")}
              </td>
              <td className="px-4 py-2">
                <Dropdown
                  options={Object.keys(WINDOWS_ECMA_KEYMAP)}
                  onOpen={() => {
                    setActiveKeybindId([bind.id, true]);
                  }}
                  onBlur={() => cancelChangeKey(bind.id, true)}
                  onChange={(option) => {
                    const newKeybinds = binds.map((b) =>
                      b.id === bind.id
                        ? {
                          ...b,
                          input: option,
                          output: b.type === "mash_trigger" ? option : b.output
                        }
                        : b,
                    );
                    setBinds(newKeybinds);
                    if (bind.type === "socd") {
                      const theseLinkedBinds = linkedBinds.find(
                        (b) => b[0] === bind.id || b[1] === bind.id,
                      );
                      setSocdLinkedBinds(
                        newKeybinds,
                        theseLinkedBinds![0],
                        theseLinkedBinds![1],
                        true,
                      );
                    }
                  }}
                  openAt={
                    bind.id === activeKeybindId?.[0] && activeKeybindId?.[1]
                      ? undefined
                      : { open: false, x: -500, y: 0 }
                  }
                >
                  {bind.input}
                </Dropdown>
              </td>
              <td className="flex justify-center gap-2.5 px-0 py-2">
                <button
                  onClick={() => {
                    if (bind.type === "mash_trigger")
                      setBinds(binds.filter(b => b.type !== "mash_trigger"));
                    else
                      setBinds(binds.filter(b => b.id !== bind.id));
                  }}
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
                        fillRule="evenodd"
                        clipRule="evenodd"
                        d="M90.914,5.296c6.927-7.034,18.188-7.065,25.154-0.068 c6.961,6.995,6.991,18.369,0.068,25.397L85.743,61.452l30.425,30.855c6.866,6.978,6.773,18.28-0.208,25.247 c-6.983,6.964-18.21,6.946-25.074-0.031L60.669,86.881L30.395,117.58c-6.927,7.034-18.188,7.065-25.154,0.068 c-6.961-6.995-6.992-18.369-0.068-25.397l30.393-30.827L5.142,30.568c-6.867-6.978-6.773-18.28,0.208-25.247 c6.983-6.963,18.21-6.946,25.074,0.031l30.217,30.643L90.914,5.296L90.914,5.296z"
                      />
                    </g>
                  </svg>
                </button>
              </td>
            </tr>
          ))}
          <tr>
            <td
              colSpan={5}
              className="cursor-pointer border-b border-indigo-950 bg-slate-800 bg-opacity-60 hover:bg-slate-500"
              onClick={(event) =>
                setNewBindDropdownOpen({
                  open: true,
                  x: event.clientX,
                  y: event.clientY,
                })
              }
              onBlur={() => setNewBindDropdownOpen({ open: false, x: 0, y: 0 })}
            >
              +
              <Dropdown
                options={binds.find(b => b.type === "mash_trigger") ? ["Keyboard", "Controller", "SOCD"] : ["Keyboard", "Controller", "SOCD", "Mash_Trigger"]}
                onChange={(option) => {
                  const type = option.toLowerCase() as BindType;
                  if (type === "socd") {
                    setBinds([
                      ...binds,
                      {
                        id: bindsCount,
                        input: "",
                        type,
                        output: "",
                      },
                      {
                        id: bindsCount + 1,
                        input: "",
                        type,
                        output: "",
                      },
                    ]);
                    setLinkedBinds([
                      ...linkedBinds,
                      [bindsCount, bindsCount + 1],
                    ]);
                    setBindsCount(bindsCount + 2);
                  } else if (type === "mash_trigger") {
                    setBinds([
                      ...binds,
                      {
                        id: bindsCount,
                        input: "",
                        type,
                        output: "",
                      },
                      {
                        id: bindsCount + 1,
                        input: "",
                        type,
                        output: "",
                      },
                      {
                        id: bindsCount + 2,
                        input: "",
                        type,
                        output: "",
                      },
                    ]);
                    setBindsCount(bindsCount + 3);
                  } else {
                    setBinds([
                      ...binds,
                      {
                        id: bindsCount,
                        input: "",
                        type,
                        output: "",
                      },
                    ]);
                    setBindsCount(bindsCount + 1);
                  }
                  setNewBindDropdownOpen({ open: false, x: 0, y: 0 });
                }}
                onBlur={() =>
                  setNewBindDropdownOpen({ open: false, x: 0, y: 0 })
                }
                hidden={true}
                openAt={newBindDropdownOpen}
              />
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
    {infoModalOpen && (
      <div className="fixed inset-0 size-auto max-h-none max-w-none overflow-y-auto bg-transparent backdrop:bg-transparent">
        <div className="fixed inset-0 bg-gray-900/50 transition-opacity data-closed:opacity-0 data-enter:duration-300 data-enter:ease-out data-leave:duration-200 data-leave:ease-in"></div>
        <div className="flex min-h-full items-end justify-center p-4 text-center focus:outline-none sm:items-center sm:p-0">
          <div className="relative transform overflow-hidden rounded-lg bg-gray-800 text-left shadow-xl transition-all data-closed:translate-y-4 data-closed:opacity-0 data-enter:duration-300 data-enter:ease-out data-leave:duration-200 data-leave:ease-in sm:my-8 sm:w-full sm:max-w-lg data-closed:sm:translate-y-0 data-closed:sm:scale-95">
            <div className="bg-gray-800 px-4 pt-5 pb-4 sm:p-6 sm:pb-4">
              <div className="sm:flex sm:items-start">
                <div className="mx-auto flex size-12 shrink-0 items-center justify-center rounded-full bg-red-500/10 sm:mx-0 sm:size-10">
                </div>
                <div className="mt-3 text-center sm:mt-0 sm:ml-4 sm:text-left">
                  <h3 id="dialog-title" className="text-base font-semibold text-white">Edit Keybinds Info</h3>
                  <div className="mt-2">
                    <p className="text-sm text-gray-200">
                      There are four different types of binds that OverBind currently supports:
                      <br/>
                      <br/>
                      <ul style={{ listStyle: "disc" }}>
                        <li>
                          <div className="flex flex-row gap-2 items-center"><h3>Keyboard</h3><InputTypeIcon type={"keyboard"}/></div>
                          <p>
                            Allows you to map one keyboard key to another. Note that the input key is on the RIGHT and the output key is on the LEFT.
                          </p>
                        </li>
                        <li>
                          <div className="flex flex-row gap-2 items-center"><h3>Controller</h3><InputTypeIcon type={"controller"}/></div>
                          <p>
                            Allows you to map one keyboard key to a controller button or trigger. Selected triggers will be pressed all the way when the input key is pressed. Joystick directions will be pressed 89% to the edge of the
                            specified direction (some apps do not behave correctly with 100% joystick directions). Note that Overbind is not capable of using a controller button as an input.
                          </p>
                        </li>
                        <li>
                          <div className="flex flex-row gap-2 items-center"><h3>SOCD</h3><InputTypeIcon type={"socd"}/></div>
                          <p>
                            Allows you to specify opposing cardinal direction keys for last pressed override during simultaneous presses. What this effectively means is that when left and right arrow keys are mapped
                            holding left and then pressing right will cause the left arrow key to be unpressed while the right arrow key is being held. In many games having both arrow keys pressed will be registered
                            as no direction and cause you to stand still. With SOCD cleaning, whatever direction you last pressed will be the direction you move. Requires two inputs to be specified, each overriding the other.
                          </p>
                        </li>
                        <li>
                          <div className="flex flex-row gap-2 items-center"><h3>Mash_Trigger</h3><InputTypeIcon type={"mash_trigger"}/></div>
                          <p>
                            [ONLY FOR HOLLOW KNIGHT] Allows you to specify the three buttons used for text mashing. Please map these keys to your focus, nail, and jump keys for this to work properly.
                            When you encounter a text box, holding all three buttons will trigger the automasher to mash through the text box for you. Please make sure you have the .DLL (windows) or .SO (linux)
                            to enable the masher, you will know it's working if you see the mash trigger icon in the top left corner of your game. The text masher will not work without it.
                          </p>
                        </li>
                      </ul>
                    </p>
                  </div>
                </div>
              </div>
            </div>
            <div className="bg-gray-700/25 px-4 py-3 sm:flex sm:flex-row-reverse sm:px-6">
              <button type="button" onClick={() => setInfoModalOpen(false)} className="mt-3 inline-flex w-full justify-center rounded-md bg-white/10 px-3 py-2 text-sm font-semibold text-white inset-ring inset-ring-white/5 hover:bg-white/20 sm:mt-0 sm:w-auto">Close</button>
            </div>
          </div>
        </div>
      </div>
    )}
    </>
  );
}

export default KeybindSettings;
