import { ReactNode, useEffect, useRef, useState } from "react";

const Dropdown = ({
  options,
  children,
  onChange,
}: {
  options: string[];
  children: ReactNode;
  onChange: (option: string) => void;
}) => {
  const [isOpen, setIsOpen] = useState(false);
  const [dropdownDirection, setDropdownDirection] = useState("down");
  const dropdownRef = useRef<HTMLDivElement>(null);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const optionsRefs = useRef<Record<string, HTMLAnchorElement>>({});
  const optionsListRef = useRef<HTMLDivElement>(null);

  const toggleDropdown = () => {
    if (!isOpen && buttonRef.current && dropdownRef.current) {
      const dropdownRect = buttonRef.current.getBoundingClientRect();
      const spaceBelow = window.innerHeight - dropdownRect.bottom;
      const spaceNeeded = 240; // max-h-60

      setDropdownDirection(spaceBelow >= spaceNeeded ? "down" : "up");
    }

    setIsOpen(!isOpen);
  };

  const updateDropdownWidth = () => {
    if (optionsListRef.current && optionsRefs.current.length) {
      let maxWidth = 0;
      Object.values(optionsRefs.current).forEach((optionElement) => {
        if (optionElement) {
          maxWidth = Math.max(maxWidth, optionElement.offsetWidth);
        }
      });
      optionsListRef.current.style.width = `${maxWidth}px`;
    }
  };

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

  useEffect(updateDropdownWidth, [options]);

  return (
    <div className="relative" ref={dropdownRef}>
      <button
        ref={buttonRef}
        className={`inline-flex justify-center gap-x-1.5 rounded-md bg-blue-900 px-4 py-2 shadow-sm hover:bg-blue-700 ${
          typeof children === "string" ? "bg-blue-900" : "bg-gray-500"
        }`}
        onClick={toggleDropdown}
      >
        {children}
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
        <div
          ref={optionsListRef}
          className={`absolute z-10 w-56 rounded-md bg-blue-900 shadow-lg ${
            dropdownDirection === "up" ? "bottom-full mb-1" : "mt-1"
          }`}
        >
          <div className="scrollbar-hide scroll overflow max-h-60 overflow-scroll rounded-md py-1 text-base">
            {options.map((option) => (
              <a
                key={option}
                href="#"
                ref={(el) => {
                  if (el) {
                    optionsRefs.current[option] = el;
                  }
                }}
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
