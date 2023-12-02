import { invoke } from "@tauri-apps/api/tauri";
import { useEffect, useState } from "react";
import KeybindSettings from "./Edit";

let init = false;

function App() {
  const runOverbind = async () => {
    try {
      const response = await invoke("start_process");
      console.log(response); // Log or handle the success response
      setErr("");
      await updateIsOverbindRunning();
    } catch (error) {
      console.error(error); // Handle the error case
      setErr(error as string);
    }
  };

  const stopOverbind = async () => {
    try {
      const response = await invoke("stop_process");
      console.log(response);
      setErr("");
      await updateIsOverbindRunning();
    } catch (error) {
      console.error(error);
      setErr(error as string);
    }
  };

  const updateIsOverbindRunning = async () => {
    try {
      const response = await invoke("is_process_running");
      setIsOverbindRunning(response as boolean);
    } catch (error) {
      console.error(error);
    }
  };

  const formatErrorMessage = (errorMessage: string) => {
    const urlRegex = /(https?:\/\/[^\s]+)/g;
    return errorMessage
      .replace(/\n/g, "<br>") // Replace newline characters with <br>
      .replace(
        urlRegex,
        (url) =>
          `<a href="${url}" target="_blank" rel="noopener noreferrer">${url}</a>`,
      );
  };

  const [isOverbindRunning, setIsOverbindRunning] = useState(false);
  const [isEditingBinds, setIsEditingBinds] = useState(false);
  const [err, setErr] = useState("");
  useEffect(() => {
    updateIsOverbindRunning().then(() => {
      if (!init) {
        init = true;
        runOverbind();
      }
    });
  }, []);

  return (
    <div className="justify-centerpt-[10vh] m-0 flex flex-col text-center">
      <div className="overcharm-bg flex h-[90px] items-center justify-center">
        <h1 className="text-center text-3xl">Welcome to OverBind!</h1>
      </div>

      {isOverbindRunning ? (
        <div className="flex items-center justify-center gap-2 text-2xl">
          <div className="h-4 w-4 rounded-full bg-green-500 shadow-[0_0_8px_2px_rgba(0,255,0,0.6)]" />
          Enabled
        </div>
      ) : (
        <div className="flex items-center justify-center gap-2 text-2xl">
          <div className="h-4 w-4 rounded-full bg-gray-500" />
          Disabled
        </div>
      )}

      <div className="mt-4 flex w-full justify-center gap-2.5">
        {!isOverbindRunning ? (
          <button
            className="font-mediumtext-white rounded-md bg-purple-500 bg-opacity-60 px-5 py-2.5
            text-base shadow outline-none transition-colors
            hover:bg-purple-600 active:bg-purple-800 active:bg-opacity-40"
            onClick={runOverbind}
          >
            Launch
          </button>
        ) : (
          <button
            className="rounded-md bg-red-500 bg-opacity-60 px-5 py-2.5 text-base font-medium
            text-white shadow outline-none transition-colors
            hover:bg-red-600 active:bg-red-800 active:bg-opacity-40"
            onClick={stopOverbind}
          >
            Stop
          </button>
        )}
        <button
          className="font-mediumtext-white rounded-md bg-yellow-500 bg-opacity-60 px-5 py-2.5
          text-base shadow outline-none transition-colors
          hover:bg-yellow-600 active:bg-yellow-800"
          onClick={() => setIsEditingBinds(!isEditingBinds)}
        >
          Edit
        </button>
      </div>

      {err && (
        <p
          className="whitespace-pre-wrap text-red-500"
          dangerouslySetInnerHTML={{ __html: formatErrorMessage(err) }}
        ></p>
      )}

      {isEditingBinds && (
        <KeybindSettings
          onCancel={() => setIsEditingBinds(false)}
          onSave={async () => {
            setIsEditingBinds(false);
            if (isOverbindRunning) {
              await stopOverbind();
            }
          }}
          onErr={setErr}
        />
      )}
    </div>
  );
}

export default App;
