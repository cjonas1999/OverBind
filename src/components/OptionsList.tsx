import { useState } from "react";

const OptionsList = ({
  options,
  setOptions,
}: {
  options: string[];
  setOptions: (options: string[]) => void;
}) => {
  const [isEditing, setIsEditing] = useState(false);
  const [newOptionName, setNewOptionName] = useState("");

  const handleAddNewOption = () => {
    setIsEditing(true);
  };

  const handleSaveNewOption = () => {
    if (newOptionName.trim() !== "") {
      const newOptions = [...options, newOptionName];
      setOptions(newOptions);
      setNewOptionName("");
      setIsEditing(false);
    }
  };

  return (
    <div className="flex flex-wrap justify-center gap-2">
      {options.map((program, index) => (
        <div
          key={index}
          className="inline-flex cursor-pointer justify-center gap-x-1.5 rounded-md bg-blue-900 px-4 py-2 shadow-sm hover:bg-blue-700 hover:line-through"
          onClick={() => {
            const updatedOptions = options.filter((_, i) => i !== index);
            setOptions(updatedOptions);
          }}
        >
          <span>{program}</span>
        </div>
      ))}
      {isEditing ? (
        <input
          autoFocus
          type="text"
          value={newOptionName}
          onChange={(e) => setNewOptionName(e.target.value)}
          onBlur={handleSaveNewOption}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              handleSaveNewOption();
            }
          }}
          className="inline-flex cursor-pointer justify-center gap-x-1.5 rounded-md bg-blue-900 px-4 py-2 shadow-sm hover:bg-blue-700"
        />
      ) : (
        <div
          className="inline-flex cursor-pointer justify-center gap-x-1.5 rounded-md bg-blue-900 px-4 py-2 shadow-sm hover:bg-blue-700"
          onClick={handleAddNewOption}
        >
          <span>+</span>
        </div>
      )}
    </div>
  );
};

export default OptionsList;
