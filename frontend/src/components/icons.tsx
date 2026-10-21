import type { ReactNode, SVGProps } from 'react'

type IconProps = SVGProps<SVGSVGElement>

function Icon({ children, viewBox = '0 0 24 24', ...rest }: IconProps & { children: ReactNode; viewBox?: string }) {
  return (
    <svg
      viewBox={viewBox}
      fill="none"
      stroke="currentColor"
      strokeWidth={1.8}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      {...rest}
    >
      {children}
    </svg>
  )
}

export function PlusIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M12 5v14M5 12h14" />
    </Icon>
  )
}

export function SendIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M22 2L11 13M22 2l-7 20-4-9-9-4 20-7z" />
    </Icon>
  )
}

export function PaperclipIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M21.44 11.05l-9.19 9.19a6 6 0 0 1-8.49-8.49l9.19-9.19a4 4 0 0 1 5.66 5.66l-9.2 9.19a2 2 0 0 1-2.83-2.83l8.49-8.48" />
    </Icon>
  )
}

export function PanelLeftIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M9 3v18" />
    </Icon>
  )
}

export function PanelRightIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M15 3v18" />
    </Icon>
  )
}

export function XIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M18 6L6 18M6 6l12 12" />
    </Icon>
  )
}

export function CheckIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M20 6L9 17l-5-5" />
    </Icon>
  )
}

export function ChevronDownIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M6 9l6 6 6-6" />
    </Icon>
  )
}

export function DownloadIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4M7 10l5 5 5-5M12 15V3" />
    </Icon>
  )
}

export function ChartIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M3 3v18h18" />
      <path d="M7 14l4-5 4 3 5-6" />
    </Icon>
  )
}

export function FileIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <path d="M14 2v6h6" />
    </Icon>
  )
}

export function DocIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
      <path d="M14 2v6h6M9 13h6M9 17h4" />
    </Icon>
  )
}

export function TableIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <rect x="3" y="3" width="18" height="18" rx="2" />
      <path d="M3 9h18M3 15h18M9 3v18" />
    </Icon>
  )
}

export function TrashIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M3 6h18M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6" />
      <path d="M10 11v6M14 11v6" />
    </Icon>
  )
}

export function PencilIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <Icon className={className} {...rest}>
      <path d="M12 20h9" />
      <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4z" />
    </Icon>
  )
}

export function MoreIcon({ className = 'h-4 w-4', ...rest }: IconProps) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" className={className} {...rest}>
      <circle cx="5" cy="12" r="1.6" />
      <circle cx="12" cy="12" r="1.6" />
      <circle cx="19" cy="12" r="1.6" />
    </svg>
  )
}
