import { invoke } from "@tauri-apps/api";
import { useEffect, useState } from "react";

function SettingsModal({
  onCancel,
  onSave,
  onErr,
}: {
  onCancel: () => void;
  onSave: () => void;
  onErr: (error: string) => void;
}) {
  const [closeToTray, setcloseToTray] = useState(false);

  const saveSettings = () => {
    const settingsToSave = {
      close_to_tray: closeToTray,
    };

    invoke("save_app_settings", { settings: settingsToSave })
      .then(() => onSave())
      .catch((err) => onErr(err));
  };

  const readSettings = () => {
    invoke("read_app_settings").then((response: any) => {
      console.log(response);
      console.log(JSON.stringify(response));

      setcloseToTray(response.close_to_tray);
    });
  };

  useEffect(readSettings, []);

  return (
    <div className="p-4 text-white">
      <h1 className="mb-4 text-lg font-bold">Edit Settings</h1>
      <div className="flex h-full flex-col items-center justify-between rounded-sm p-3">
        <div className="flex w-6/12 justify-evenly bg-slate-700 p-6">
          <label className="cursor-pointer">
            <input
              type="checkbox"
              checked={closeToTray}
              onChange={() => setcloseToTray(!closeToTray)}
              className="mx-1.5 h-4 w-4"
            />
            Close to system tray
          </label>
        </div>
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
      </div>
    </div>
  );
}

export default SettingsModal;
