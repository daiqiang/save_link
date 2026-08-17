// 内联 lucide 风格图标（零依赖 SVG），与原型图保持一致。
import type { JSX } from "react";

type P = { size?: number };
const base = (size: number) => ({
  width: size, height: size, viewBox: "0 0 24 24", fill: "none",
  stroke: "currentColor", strokeWidth: 2, strokeLinecap: "round" as const,
  strokeLinejoin: "round" as const,
});

export const Icon = {
  Search: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><circle cx="11" cy="11" r="8" /><path d="m21 21-4.3-4.3" /></svg>
  ),
  Plus: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M5 12h14M12 5v14" /></svg>
  ),
  Camera: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M14.5 4h-5L7 7H4a2 2 0 0 0-2 2v9a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V9a2 2 0 0 0-2-2h-3l-2.5-3z" /><circle cx="12" cy="13" r="3" /></svg>
  ),
  RotateCcw: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M3 12a9 9 0 1 0 3-6.7L3 8" /><path d="M3 3v5h5" /></svg>
  ),
  Lock: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 10 0v4" /></svg>
  ),
  Unlock: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><rect x="3" y="11" width="18" height="11" rx="2" /><path d="M7 11V7a5 5 0 0 1 9.9-1" /></svg>
  ),
  More: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><circle cx="5" cy="12" r="1" /><circle cx="12" cy="12" r="1" /><circle cx="19" cy="12" r="1" /></svg>
  ),
  Settings: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" /></svg>
  ),
  Help: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><circle cx="12" cy="12" r="10" /><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3" /><path d="M12 17h.01" /></svg>
  ),
  Folder: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2z" /></svg>
  ),
  FileSearch: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h7" /><path d="M14 2v6h6" /><path d="M20 13.5V8l-6-6" /><circle cx="16" cy="17" r="3" /><path d="m18.5 19.5 2.5 2.5" /></svg>
  ),
  Gamepad: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M6 11h4M8 9v4M15 12h.01M18 10h.01" /><path d="M7 6h10a4 4 0 0 1 3.8 2.75l1.1 3.6A3.5 3.5 0 0 1 18.55 17a3 3 0 0 1-2.12-.88L14.5 14.2h-5l-1.93 1.93A3 3 0 0 1 5.45 17a3.5 3.5 0 0 1-3.35-4.65l1.1-3.6A4 4 0 0 1 7 6Z" /></svg>
  ),
  Copy: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><rect x="9" y="9" width="13" height="13" rx="2" /><path d="M5 15H4a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h9a2 2 0 0 1 2 2v1" /></svg>
  ),
  CloudUpload: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M12 13v8" /><path d="m16 17-4-4-4 4" /><path d="M4.36 15.36A8 8 0 1 1 18.53 9.5H20a4 4 0 0 1 0 8h-2" /></svg>
  ),
  Download: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M12 3v12" /><path d="m7 10 5 5 5-5" /><path d="M5 21h14" /></svg>
  ),
  Edit: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M11 4H4a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h14a2 2 0 0 0 2-2v-7" /><path d="M18.5 2.5a2.12 2.12 0 0 1 3 3L12 15l-4 1 1-4z" /></svg>
  ),
  Save: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M19 21H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11l5 5v11a2 2 0 0 1-2 2z" /><path d="M17 21v-8H7v8M7 3v5h8" /></svg>
  ),
  Shield: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" /></svg>
  ),
  Check: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M20 6 9 17l-5-5" /></svg>
  ),
  CheckCircle: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" /><path d="m22 4-10 10.01-3-3" /></svg>
  ),
  Alert: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3z" /><path d="M12 9v4M12 17h.01" /></svg>
  ),
  Close: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M18 6 6 18M6 6l12 12" /></svg>
  ),
  Trash: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" /></svg>
  ),
  History: ({ size = 16 }: P): JSX.Element => (
    <svg {...base(size)}><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8" /><path d="M3 3v5h5M12 7v5l4 2" /></svg>
  ),
};
