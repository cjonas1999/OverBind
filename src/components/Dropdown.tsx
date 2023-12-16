import { useEffect, useRef, useState } from "react";

const Dropdown = ({
  options,
  selected,
  onChange,
}: {
  options: string[];
  selected: string;
  onChange: (option: string) => void;
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(event.target as HTMLElement)
      ) {
        setIsOpen(false);
      }
    };

    document.addEventListener("mousedown", handleClickOutside);
    return () => {
      document.removeEventListener("mousedown", handleClickOutside);
    };
  }, [dropdownRef]);

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        className="inline-flex w-full justify-center gap-x-1.5 rounded-md bg-blue-900 px-4 py-2  shadow-sm hover:bg-blue-800"
        onClick={() => setIsOpen(!isOpen)}
      >
        {selected}
        <svg
          className="-mr-1 h-5 w-5 text-white"
          viewBox="0 0 20 20"
          fill="currentColor"
          aria-hidden="true"
        >
          <path
            fillRule="evenodd"
            d="M5.23 7.21a.75.75 0 011.06.02L10 11.168l3.71-3.938a.75.75 0 111.08 1.04l-4.25 4.5a.75.75 0 01-1.08 0l-4.25-4.5a.75.75 0 01.02-1.06z"
            clipRule="evenodd"
          />
        </svg>
      </button>

      {isOpen && (
        <div className="absolute z-10 mt-1 w-full rounded-md bg-blue-900 shadow-lg">
          <div className="scrollbar-hide scroll overflow max-h-60 overflow-scroll rounded-md py-1 text-base">
            {options.map((option) => (
              <a
                key={option}
                href="#"
                className="block px-4 py-2 text-sm text-white hover:bg-blue-800"
                onClick={(e) => {
                  e.preventDefault();
                  onChange(option);
                  setIsOpen(false);
                }}
              >
                {option}
              </a>
            ))}
          </div>
        </div>
      )}
    </div>
  );
};

export default Dropdown;
