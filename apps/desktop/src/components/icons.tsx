import type { ReactNode } from "react";

/** One 16 px stroke icon set, drawn in the current text colour. Always decorative: the button names the action. */
function Icon({ children, size = 16 }: { children: ReactNode; size?: number }) {
  return (
    <svg className="icon" viewBox="0 0 16 16" width={size} height={size} aria-hidden="true" focusable="false">
      {children}
    </svg>
  );
}

export const SaveIcon = () => (
  <Icon>
    <path d="M3 2.5h8l2.5 2.5v8.5H2.5v-11z" />
    <path d="M5 2.5v3.5h5V2.5M5 13.5v-4h6v4" />
  </Icon>
);

export const QueueIcon = () => (
  <Icon>
    <path d="M3 4h10M3 8h10M3 12h6" />
  </Icon>
);

export const SettingsIcon = () => (
  <Icon>
    <path d="M13.06 6.43 L14.80 6.83 L14.80 9.17 L13.06 9.57 L12.69 10.47 L13.63 11.98 L11.98 13.63 L10.47 12.69 L9.57 13.06 L9.17 14.80 L6.83 14.80 L6.43 13.06 L5.53 12.69 L4.02 13.63 L2.37 11.98 L3.31 10.47 L2.94 9.57 L1.20 9.17 L1.20 6.83 L2.94 6.43 L3.31 5.53 L2.37 4.02 L4.02 2.37 L5.53 3.31 L6.43 2.94 L6.83 1.20 L9.17 1.20 L9.57 2.94 L10.47 3.31 L11.98 2.37 L13.63 4.02 L12.69 5.53Z" />
    <circle cx="8" cy="8" r="2.1" />
  </Icon>
);

export const ChevronDownIcon = () => (
  <Icon size={12}>
    <path d="M4 6l4 4 4-4" />
  </Icon>
);

export const CloseIcon = () => (
  <Icon>
    <path d="M4 4l8 8M12 4l-8 8" />
  </Icon>
);

export const SearchIcon = () => (
  <Icon>
    <circle cx="7" cy="7" r="4.5" />
    <path d="M10.5 10.5l3 3" />
  </Icon>
);

export const RefreshIcon = () => (
  <Icon>
    <path d="M13 8a5 5 0 1 1-1.5-3.6M13 2.5v2.8h-2.8" />
  </Icon>
);

export const PrinterIcon = () => (
  <Icon>
    <path d="M4.5 6V2.5h7V6" />
    <rect x="2" y="6" width="12" height="6" rx="2" />
    <path d="M4.5 10.5h7v3h-7z" />
  </Icon>
);

export const PaletteIcon = () => (
  <Icon>
    <circle cx="8" cy="8" r="5.5" />
    <path d="M8 2.5a5.5 5.5 0 0 1 0 11z" fill="currentColor" />
  </Icon>
);

export const PulseIcon = () => (
  <Icon>
    <path d="M1.5 8.5h3l2-5 3 9 2-4h3" />
  </Icon>
);

export const MinusIcon = () => (
  <Icon size={12}>
    <path d="M3.5 8h9" />
  </Icon>
);

export const PlusIcon = () => (
  <Icon size={12}>
    <path d="M3.5 8h9M8 3.5v9" />
  </Icon>
);

export const OpenIcon = () => (
  <Icon>
    <path d="M2 4.5V12.5h12V6H8L6.5 4.5z" />
  </Icon>
);

export const RotateIcon = () => (
  <Icon>
    <path d="M3 8a5 5 0 1 0 1.5-3.6M3 2.5v2.8h2.8" />
  </Icon>
);
