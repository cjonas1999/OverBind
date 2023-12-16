import { invoke } from "@tauri-apps/api/tauri";
import { useEffect, useState } from "react";
import KeybindSettings from "./components/Edit";

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
  const [err, setErr] = useState("");

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
          onClick={() => setIsEditingBinds(!isEditingBinds)}
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

      {isLogVisible && (
        <div className="scrollbar-hide scroll overflow mx-12 mt-10 h-80 overflow-scroll bg-zinc-900 p-5 text-left font-mono">
          {logs
            .slice()
            .reverse()
            .map((log, i) => (
              <div
                key={i}
                className={`mb-2 ${
                  log.type === "log"
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
