import { invoke } from "@tauri-apps/api/tauri";

const runCppExecutable = () => {
  invoke('run_key_to_controller')
    .then(response => {
      console.log(response); // Log or handle the success response
    })
    .catch(error => {
      console.error(error); // Handle the error case
    });
};

function App() {
  

  return (
    <div className="container">
      <div className="title">
        <h1>Welcome to OverBind!</h1>
      </div>

      <p>Click on button to launch the overbind executable</p>

      <div className="buttons">
        <button onClick={runCppExecutable}>Launch</button>
      </div>
    </div>
  );
}

export default App;
