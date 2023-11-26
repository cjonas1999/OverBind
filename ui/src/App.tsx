import { invoke } from "@tauri-apps/api/tauri";
import { useEffect, useState } from "react";

function App() {
  const runOverbind = () => {
    invoke("start_process")
      .then((response) => {
        console.log(response); // Log or handle the success response
        setErr("");
        updateIsOverbindRunning();
      })
      .catch((error) => {
        console.error(error); // Handle the error case
        setErr(error);
      });
  };

  const stopOverbind = () => {
    invoke("stop_process")
      .then((response) => {
        console.log(response);
        setErr("");
        updateIsOverbindRunning();
      })
      .catch((error) => {
        console.error(error);
        setErr(error);
      });
  };

  const updateIsOverbindRunning = () => {
    invoke("is_process_running")
      .then((response) => setIsOverbindRunning(response as boolean))
      .catch((err) => console.error(err));
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
  const [err, setErr] = useState("");
  useEffect(() => {
    updateIsOverbindRunning();
  }, []);

  return (
    <div className="justify-centerpt-[10vh] m-0 flex flex-col text-center">
      <div className="overcharm-bg flex h-[90px] items-center justify-center">
        <h1 className="text-center text-3xl">Welcome to OverBind!</h1>
      </div>

      {isOverbindRunning ? (
        <p>Overbind executable currently running</p>
      ) : (
        <p>Click on button to launch the Overbind executable</p>
      )}

      <div className="flex w-full justify-center gap-2.5">
        <button
          className="font-mediumtext-white rounded-md bg-black bg-opacity-60 px-5 py-2.5
            text-base shadow outline-none transition-colors
            hover:border-blue-600 active:bg-black active:bg-opacity-40"
          onClick={runOverbind}
        >
          Launch
        </button>
        <button
          className="rounded-md bg-black bg-opacity-60 px-5 py-2.5 text-base font-medium
            text-white shadow outline-none transition-colors
            hover:border-blue-600 active:bg-black active:bg-opacity-40"
          onClick={stopOverbind}
        >
          Stop
        </button>
      </div>

      {err && (
        <p
          className="whitespace-pre-wrap text-red-500"
          dangerouslySetInnerHTML={{ __html: formatErrorMessage(err) }}
        ></p>
      )}
    </div>
  );
}

export default App;
