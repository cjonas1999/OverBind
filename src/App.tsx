import { invoke } from "@tauri-apps/api/tauri";
import { useEffect, useState } from "react";
import KeybindSettings from "./components/Edit";
import SettingsModal from "./components/SettingsModal";
import { listen } from "@tauri-apps/api/event";

let init = false;

type LogEntry = {
  type: "log" | "error" | "warn"; // Add more types as needed
  message: string;
  timestamp: number;
};

function App() {
  const runOverbind = async () => {
    try {
      const response = await invoke("start_interception");
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
      const response = await invoke("stop_interception");
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
      const response = await invoke("is_interceptor_running");
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
  const [isEditingSettings, setEditingSettings] = useState(false);
  const [err, setErr] = useState("");
  const [isDirty, setIsDirty] = useState(false);
  const [isSettingsIncomplete, setIsSettingsIncomplete] = useState(false);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [isLogVisible, setIsLogVisible] = useState(false);

  const toggleLogs = () => {
    console.log(`${!isLogVisible ? "Enabling" : "Disabling"} console logs`);
    setIsLogVisible(!isLogVisible);
  };

  useEffect(() => {
    const originalConsoleLog = console.log;
    console.log = (...args) => {
      setLogs((prevLogs) => [
        ...prevLogs,
        { type: "log", message: args.join(" "), timestamp: Date.now() },
      ]);
      originalConsoleLog(...args);
    };

    const originalConsoleError = console.error;
    console.error = (...args) => {
      setLogs((prevLogs) => [
        ...prevLogs,
        { type: "error", message: args.join(" "), timestamp: Date.now() },
      ]);
      originalConsoleError(...args);
    };

    const originalConsoleWarn = console.warn;
    console.warn = (...args) => {
      setLogs((prevLogs) => [
        ...prevLogs,
        { type: "warn", message: args.join(" "), timestamp: Date.now() },
      ]);
      originalConsoleWarn(...args);
    };

    return () => {
      console.log = originalConsoleLog;
      // Reset other console methods if overridden
    };
  }, []);

  // update ui when tray menu is used
  useEffect(() => {

    // Listen for panic events from the backend
    listen('panic', (event) => {
      const panicMessage = event.payload;
      console.error("Panic occurred:", panicMessage);
    });


    listen("tray_intercept_disable", () => {
      setIsOverbindRunning(false);
    });
    listen("tray_intercept_enable", () => {
      setIsOverbindRunning(true);
    });
    listen("settings_incomplete", (event) => {
      const newIsSettingsIncomplete = event.payload as boolean;
      setIsSettingsIncomplete(newIsSettingsIncomplete);
      if (newIsSettingsIncomplete) {
        stopOverbind();
      }
    });

    // Wait for 100ms to ensure listeners are registered
    setTimeout(() => {
      if (!init) {
        init = true;
        runOverbind();
      }
    }, 100);
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

      {isDirty && (
        <div className="text-red-500">
          <p>Please restart Overbind for your changes to take effect.</p>
        </div>
      )}

      {!isDirty && isSettingsIncomplete && (
        <div className="text-red-500">
          <p>There are some settings that need to be configured for Overbind to run correctly.
            Please go do Settings and configure them.</p>
        </div>
      )}

      <div className="mt-4 flex w-full justify-center gap-2.5">
        {!isOverbindRunning ? (
          <button
            className="rounded-md bg-purple-500 bg-opacity-60 px-5 py-2.5 text-base font-medium
            text-white shadow outline-none transition-colors
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
          className="rounded-md bg-yellow-500 bg-opacity-60 px-5 py-2.5 text-base font-medium
          text-white shadow outline-none transition-colors
          hover:bg-yellow-600 active:bg-yellow-800"
          onClick={() => {
            setIsEditingBinds(!isEditingBinds);
            if (isEditingSettings) {
              setEditingSettings(false);
            }
          }}
        >
          Edit
        </button>
        <button
          className="rounded-md bg-slate-800 bg-opacity-90 px-5 py-2.5 text-base font-medium
          text-white shadow outline-none transition-colors
          hover:bg-slate-600 active:bg-slate-600"
          onClick={toggleLogs}
        >
          Logs
        </button>
        <button
          className="rounded-md bg-stone-600 bg-opacity-90 px-5 py-2.5 text-base font-medium
          text-white shadow outline-none transition-colors
          hover:bg-stone-500 active:bg-stone-500"
          onClick={() => {
            setEditingSettings(!isEditingSettings);
            if (isEditingBinds) {
              setIsEditingBinds(false);
            }
          }}
        >
          Settings
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
              await runOverbind();
            }
          }}
          onErr={setErr}
        />
      )}

      {isEditingSettings && (
        <SettingsModal
          onCancel={() => setEditingSettings(false)}
          onSave={async () => {
            setEditingSettings(false);
            if (isOverbindRunning) {
              await stopOverbind();
              await runOverbind();
            }
          }}
          onDirtySave={() => setIsDirty(true)}
          onErr={setErr}
        />
      )}

      {isLogVisible && (
        <div className="scrollbar-hide scroll overflow mx-12 my-10 h-80 overflow-scroll bg-zinc-900 p-5 text-left font-mono">
          {logs
            .slice()
            .reverse()
            .map((log, i) => (
              <div
                key={i}
                className={`mb-2 ${log.type === "log"
                  ? "text-blue-500"
                  : log.type === "warn"
                    ? "text-yellow-500"
                    : log.type === "error"
                      ? "text-red-500"
                      : "text-white"
                  }`}
              >
                {log.timestamp}. {log.message}
              </div>
            ))}
        </div>
      )}
    </div>
  );
}

export default App;
