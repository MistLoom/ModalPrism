// 单色线性图标。统一 16px、1.4 描边，跟随文字颜色。
import type { SVGProps } from "react";

type Props = SVGProps<SVGSVGElement> & { size?: number };

function Icon({ size = 16, children, ...rest }: Props) {
  return (
    <svg
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.4}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  );
}

export const IconOverview = (props: Props) => (
  <Icon {...props}>
    <path d="M2.5 6.2 8 2.6l5.5 3.6v5.9L8 15l-5.5-2.9z" />
    <path d="M2.5 6.2 8 9.1l5.5-2.9M8 9.1V15" />
  </Icon>
);

export const IconSource = (props: Props) => (
  <Icon {...props}>
    <circle cx="8" cy="8" r="1.4" fill="currentColor" stroke="none" />
    <path d="M4.4 4.4a5.1 5.1 0 0 0 0 7.2M11.6 4.4a5.1 5.1 0 0 1 0 7.2" />
    <path d="M2.1 2.4a8 8 0 0 0 0 11.2M13.9 2.4a8 8 0 0 1 0 11.2" opacity=".55" />
  </Icon>
);

export const IconLibrary = (props: Props) => (
  <Icon {...props}>
    <path d="M2.6 3.4h7.2v10.2H2.6zM9.8 5.2h3.6v8.4H9.8z" />
    <path d="M4.6 5.8h3.2M4.6 8.2h3.2M4.6 10.6h2" opacity=".6" />
  </Icon>
);

export const IconChannel = (props: Props) => (
  <Icon {...props}>
    <path d="M2.4 8h3.2M10.4 8h3.2" />
    <circle cx="8" cy="8" r="2.2" />
    <path d="M2.4 5.2v5.6M13.6 5.2v5.6" opacity=".6" />
  </Icon>
);

export const IconConfig = (props: Props) => (
  <Icon {...props}>
    <path d="M2.4 4.6h11.2M2.4 8h11.2M2.4 11.4h11.2" />
    <circle cx="5.6" cy="4.6" r="1.5" fill="var(--paper-raised, #fff)" />
    <circle cx="10.2" cy="8" r="1.5" fill="var(--paper-raised, #fff)" />
    <circle cx="6.8" cy="11.4" r="1.5" fill="var(--paper-raised, #fff)" />
  </Icon>
);

export const IconSearch = (props: Props) => (
  <Icon {...props}>
    <circle cx="7.2" cy="7.2" r="4.2" />
    <path d="m10.4 10.4 3 3" />
  </Icon>
);

export const IconCommand = (props: Props) => (
  <Icon {...props}>
    <path d="M5.6 2.4a1.8 1.8 0 1 1 1.8 1.8v7.6a1.8 1.8 0 1 1-1.8-1.8h4.8a1.8 1.8 0 1 1-1.8 1.8V4.2a1.8 1.8 0 1 1 1.8 1.8H5.6" />
  </Icon>
);

export const IconClose = (props: Props) => (
  <Icon {...props}>
    <path d="m4 4 8 8M12 4l-8 8" />
  </Icon>
);

export const IconChevronLeft = (props: Props) => (
  <Icon {...props}>
    <path d="m9.6 4.4-3.2 3.6 3.2 3.6" />
  </Icon>
);

export const IconChevronRight = (props: Props) => (
  <Icon {...props}>
    <path d="m6.4 4.4 3.2 3.6-3.2 3.6" />
  </Icon>
);

export const IconArrowRight = (props: Props) => (
  <Icon {...props}>
    <path d="M3 8h9.2M9.2 4.8 12.4 8l-3.2 3.2" />
  </Icon>
);

export const IconRefresh = (props: Props) => (
  <Icon {...props}>
    <path d="M13.2 8a5.2 5.2 0 1 1-1.6-3.7" />
    <path d="M13.4 2.6v3.2h-3.2" />
  </Icon>
);

export const IconPlay = (props: Props) => (
  <Icon {...props}>
    <path d="M5.4 3.6 12 8l-6.6 4.4z" />
  </Icon>
);

export const IconCheck = (props: Props) => (
  <Icon {...props}>
    <path d="m3.4 8.4 3 3 6.2-6.8" />
  </Icon>
);

export const IconAlert = (props: Props) => (
  <Icon {...props}>
    <path d="M8 2.6 14 13H2z" />
    <path d="M8 6.4v3M8 11.2v.1" />
  </Icon>
);

export const IconClock = (props: Props) => (
  <Icon {...props}>
    <circle cx="8" cy="8" r="5.6" />
    <path d="M8 4.8V8l2.2 1.4" />
  </Icon>
);

export const IconShield = (props: Props) => (
  <Icon {...props}>
    <path d="M8 2.4 13 4v4c0 3-2.2 5-5 5.6C5.2 13 3 11 3 8V4z" />
    <path d="m6 8 1.5 1.5L10.4 6.6" opacity=".7" />
  </Icon>
);

export const IconBranch = (props: Props) => (
  <Icon {...props}>
    <circle cx="4.4" cy="4" r="1.6" />
    <circle cx="4.4" cy="12" r="1.6" />
    <circle cx="11.6" cy="8" r="1.6" />
    <path d="M4.4 5.6v4.8M6 4h2.4a1.6 1.6 0 0 1 1.6 1.6v1M6 12h2.4a1.6 1.6 0 0 0 1.6-1.6V9.4" />
  </Icon>
);

export const IconCopy = (props: Props) => (
  <Icon {...props}>
    <path d="M5.6 5.6h7v7.2h-7z" />
    <path d="M3.4 10.4V3.2h7.2" opacity=".6" />
  </Icon>
);

export const IconLayers = (props: Props) => (
  <Icon {...props}>
    <path d="m8 2.4 6 3.1-6 3.1-6-3.1z" />
    <path d="m2.6 8.6 5.4 2.8 5.4-2.8" opacity=".7" />
    <path d="m2.6 11.4 5.4 2.8 5.4-2.8" opacity=".45" />
  </Icon>
);

export const IconFile = (props: Props) => (
  <Icon {...props}>
    <path d="M4 2.6h5.2L12.6 6v7.4H4z" />
    <path d="M9 2.6V6h3.6" opacity=".6" />
  </Icon>
);

export const IconSave = (props: Props) => (
  <Icon {...props}>
    <path d="M3 3h8l2 2v8H3z" />
    <path d="M5.4 3v3h4.4V3M5.4 13v-4h5.2v4" opacity=".6" />
  </Icon>
);

export const IconTrash = (props: Props) => (
  <Icon {...props}>
    <path d="M3.4 4.4h9.2M6.4 4.4V3h3.2v1.4M4.4 4.4l.6 8.6h6l.6-8.6" />
  </Icon>
);

export const IconTop = (props: Props) => (
  <Icon {...props}>
    <path d="M4 8.4 8 4.4l4 4M8 4.6V13" />
  </Icon>
);

export const IconPause = (props: Props) => (
  <Icon {...props}>
    <path d="M5.6 3.6v8.8M10.4 3.6v8.8" />
  </Icon>
);

export const IconLive = (props: Props) => (
  <Icon {...props}>
    <circle cx="8" cy="8" r="2.4" />
    <circle cx="8" cy="8" r="5.6" opacity=".4" />
  </Icon>
);

export const IconQueue = (props: Props) => (
  <Icon {...props}>
    <path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h6.5" />
  </Icon>
);
