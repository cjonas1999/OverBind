import { invoke } from "@tauri-apps/api";
import { useEffect, useState } from "react";
import isEqual from "lodash/isEqual";
import OptionsList from "./OptionsList";

interface Setting {
  key: string;
  name: string;
  value: boolean | string[];
}

const settingNames = {
  close_to_tray: "Close to system tray",
  allowed_programs: "Allowed programs",
};

const dirtySettings = ["allowed_programs"];

function SettingsModal({
  onCancel,
  onSave,
  onDirtySave,
  onErr,
}: {
  onCancel: () => void;
  onSave: () => void;
  onDirtySave: () => void;
  onErr: (error: string) => void;
}) {
  const [originalSettings, setOriginalSettings] = useState({} as any);
  const [settings, setSettings] = useState([] as Setting[]);

  const saveSettings = () => {
    const settingsToSave = settings.reduce((acc, setting) => {
      acc[setting.key] = setting.value;
      return acc;
    }, {} as any);

    const isDirty = dirtySettings.some(
      (setting) => !isEqual(settingsToSave[setting], originalSettings[setting]),
    );
    if (isDirty) {
      onDirtySave();
    }

    invoke("save_app_settings", { settings: settingsToSave })
      .then(() => onSave())
      .catch((err) => onErr(err));
  };

  const readSettings = () => {
    invoke("read_app_settings").then((response: any) => {
      console.log(response);
      console.log(JSON.stringify(response));

      setOriginalSettings(response);
      setSettings(
        Object.keys(response).map((key) => {
          return {
            key,
            name: settingNames[key as keyof typeof settingNames],
            value: response[key],
          };
        }),
      );
    });
  };

  useEffect(readSettings, []);

  const getSettingChanger = (setting: Setting) => {
    if (typeof setting.value === "boolean") {
      return (
        <input
          type="checkbox"
          checked={setting.value}
          onChange={() => {
            setting.value = !setting.value;
            setSettings([...settings]);
          }}
          className="h-6 w-6 cursor-pointer"
        />
      );
    } else if (Array.isArray(setting.value)) {
      return (
        <OptionsList
          options={setting.value}
          setOptions={(newOptions) => {
            setting.value = newOptions;
            setSettings([...settings]);
          }}
        />
      );
    }
  };

  return (
    <div className="flex flex-col items-center justify-between rounded-sm p-3 text-white">
      <h1 className="mb-4 text-lg font-bold">Edit Settings</h1>
      <table className="mb-4 w-6/12 table-auto">
        <tbody>
          {settings.map((setting) => (
            <tr
              key={setting.key}
              className="border border-indigo-950 bg-indigo-800 bg-opacity-60"
            >
              <td className="p-2">{setting.name}</td>
              <td className="p-2">{getSettingChanger(setting)}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <div className="mb-2 mt-10">
        <button
          className="mr-2 rounded bg-green-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-green-700"
          onClick={saveSettings}
        >
          Save
        </button>
        <button
          className="mr-2 rounded bg-red-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-red-700"
          onClick={onCancel}
        >
          Close
        </button>
      </div>
      {/* </div> */}
    </div>
  );
}

export default SettingsModal;
