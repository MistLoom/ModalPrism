import type { ReactNode, SVGProps } from "react";

function Icon({ children, size = 18, ...rest }: SVGProps<SVGSVGElement> & { children: ReactNode; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.7}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  );
}

export const IconOverview = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M4 13h6V4H4zM14 20h6v-9h-6zM4 20h6v-4H4zM14 8h6V4h-6z" />
  </Icon>
);
export const IconSources = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M5 6h14M5 12h14M5 18h9" />
    <circle cx="3" cy="6" r=".6" />
    <circle cx="3" cy="12" r=".6" />
    <circle cx="3" cy="18" r=".6" />
  </Icon>
);
export const IconTasks = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M7 4h10l3 4v12H4V8z" />
    <path d="M8 12h8M8 16h5" />
  </Icon>
);
export const IconContent = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M6 3h9l4 4v14H6z" />
    <path d="M14 3v5h5M9 13h6M9 17h6" />
  </Icon>
);
export const IconDelivery = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M21 4 3 11l6 2.5L11.5 20z" />
    <path d="M21 4 9.5 13.5" />
  </Icon>
);
export const IconBell = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M6 9a6 6 0 0 1 12 0c0 4 1.5 5.5 2 6H4c.5-.5 2-2 2-6z" />
    <path d="M10 19a2 2 0 0 0 4 0" />
  </Icon>
);
export const IconPlug = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M9 3v5M15 3v5" />
    <path d="M6 8h12v3a6 6 0 0 1-12 0z" />
    <path d="M12 17v4" />
  </Icon>
);
export const IconLogs = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M4 5h16M4 10h10M4 15h16M4 20h8" />
  </Icon>
);
export const IconSettings = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M4 6h8M16 6h4M4 12h4M12 12h8M4 18h10M18 18h2" />
    <circle cx="14" cy="6" r="2" />
    <circle cx="10" cy="12" r="2" />
    <circle cx="16" cy="18" r="2" />
  </Icon>
);

export const IconSearch = (props: { size?: number }) => (
  <Icon {...props}>
    <circle cx="11" cy="11" r="6" />
    <path d="m16 16 4 4" />
  </Icon>
);
export const IconPlus = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M12 5v14M5 12h14" />
  </Icon>
);
export const IconRefresh = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M20 6v5h-5M4 18v-5h5" />
    <path d="M18.4 9A7 7 0 0 0 6.2 7.2M5.6 15A7 7 0 0 0 17.8 16.8" />
  </Icon>
);
export const IconClose = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="m6 6 12 12M18 6 6 18" />
  </Icon>
);
export const IconChevronRight = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="m9 5 7 7-7 7" />
  </Icon>
);
export const IconChevronLeft = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M15 5 8 12l7 7" />
  </Icon>
);
export const IconChevronDown = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="m5 9 7 7 7-7" />
  </Icon>
);
export const IconMenu = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M4 7h16M4 12h16M4 17h16" />
  </Icon>
);
export const IconPlay = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M7 4.5 19 12 7 19.5z" />
  </Icon>
);
export const IconPause = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M8 5v14M16 5v14" />
  </Icon>
);
export const IconTrash = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M4 7h16M9 7V4h6v3M7 7l1 13h8l1-13M10 11v6M14 11v6" />
  </Icon>
);
export const IconPencil = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M4 20h4L20 8l-4-4L4 16v4ZM14 6l4 4" />
  </Icon>
);
export const IconRestore = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M4 10a8 8 0 1 1 2.3 5.7M4 5v5h5" />
  </Icon>
);
export const IconExternal = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M14 4h6v6M20 4 11 13M19 14v6H5V6h5" />
  </Icon>
);
export const IconCheck = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="m5 12 4 4L19 6" />
  </Icon>
);
export const IconAlert = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M12 3 2.5 20h19z" />
    <path d="M12 9v5M12 17h.01" />
  </Icon>
);
export const IconMore = (props: { size?: number }) => (
  <Icon {...props}>
    <circle cx="5" cy="12" r="1" />
    <circle cx="12" cy="12" r="1" />
    <circle cx="19" cy="12" r="1" />
  </Icon>
);
export const IconDownload = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M12 4v11M8 11l4 4 4-4M5 20h14" />
  </Icon>
);
export const IconUpload = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M12 20V9M8 13l4-4 4 4M5 4h14" />
  </Icon>
);
export const IconFlask = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M9 3h6M10 3v6L4 20h16L14 9V3" />
    <path d="M7.5 15h9" />
  </Icon>
);
export const IconHistory = (props: { size?: number }) => (
  <Icon {...props}>
    <path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1" />
    <path d="M3 4v5h5" />
    <path d="M12 7.5V12l3 2" />
  </Icon>
);
