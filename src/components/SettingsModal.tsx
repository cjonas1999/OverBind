import { invoke } from "@tauri-apps/api";
import { useEffect, useState } from 'react';

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
                "close_to_tray" : closeToTray
            }

        invoke("save_app_settings", { settings: settingsToSave })
            .then(() => onSave())
            .catch((err) => onErr(err));
    }

    const readSettings = () => {
        invoke("read_app_settings")
        .then((response: any) => {
            console.log(response);
            console.log(JSON.stringify(response));

            setcloseToTray(response.close_to_tray);
        })
    }

    useEffect(readSettings, []);

    return (
        <div>
            <div className="justify-center items-center flex fixed inset-0 z-50 my-20 mx-40 h-fit">
                <div className="rounded-sm bg-slate-700 w-full h-full p-3 flex flex-col justify-between">
                    <div>
                        <h3 className="text-3xl font-semibold object-top mb-4">Settings</h3>
                        <label>
                            <input type="checkbox" checked={closeToTray} onChange={() => setcloseToTray(!closeToTray)} className="w-4 h-4 mx-1.5"/>
                            Close to system tray
                        </label>
                    </div>
                    <div className="mt-10 mb-2">
                        <button 
                        className="mr-2 rounded bg-green-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-green-700"
                        onClick={saveSettings}>
                            Save
                        </button>
                        <button
                        className="mr-2 rounded bg-red-500 bg-opacity-60 px-4 py-2 font-bold text-white hover:bg-red-700"
                        onClick={onCancel}>
                            Close
                        </button>
                    </div>
                </div>
            </div>
            <div className="bg-black opacity-30 fixed h-full w-full inset-0 z-40">
            </div>
        </div>
    )
}

export default SettingsModal;