import { invoke } from "@tauri-apps/api/tauri";
import { useEffect, useState } from "react";

function App() {

  const runOverbind = () => {
    invoke('start_process')
      .then(response => {
        console.log(response); // Log or handle the success response
        setErr('');
      })
      .catch(error => {
        console.error(error); // Handle the error case
        setErr(error);
      });
  };

  const stopOverbind = () => {
    invoke('stop_process')
      .then(response => {
        console.log(response);
        setErr('');
      })
      .catch(error => {
        console.error(error);
        setErr(error);
      })
  }

  const formatErrorMessage = (errorMessage: string) => {
    const urlRegex = /(https?:\/\/[^\s]+)/g;
    return errorMessage
      .replace(/\n/g, '<br>') // Replace newline characters with <br>
      .replace(urlRegex, url => `<a href="${url}" target="_blank" rel="noopener noreferrer">${url}</a>`);
  }  

  const [isOverbindRunning, setIsOverbindRunning] = useState(false);
  const [err, setErr] = useState('');
  useEffect(() => {
    invoke('is_process_running')
      .then(response => setIsOverbindRunning(response as boolean))
      .catch(err => console.error(err));
  }, [])

  return (
    <div className="container">
      <div className="title">
        <h1>Welcome to OverBind!</h1>
      </div>

      {isOverbindRunning ? (
        <p>Overbind executable currently running</p>
      ) : (
        <p>Click on button to launch the Overbind executable</p>
      )}

      <div className="buttons">
        <button onClick={runOverbind}>Launch</button>
      </div>

      <div className="buttons">
        <button onClick={stopOverbind}>Stop</button>
      </div>
      
      {err && (
        <p className="err" dangerouslySetInnerHTML={{ __html: formatErrorMessage(err) }}></p>
    )}
    </div>
  );
}

export default App;
