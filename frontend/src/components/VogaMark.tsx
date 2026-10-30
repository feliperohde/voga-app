/** The Voga mark: a V drawn as three connected points (src-tauri/icons/source/voga-mark.svg). */
export function VogaMark({ size = 24, className = '' }: { size?: number; className?: string }) {
  return (
    <svg width={size} height={size} viewBox="64 64 896 896" className={className} aria-hidden="true">
      <rect x="64" y="64" width="896" height="896" rx="200" fill="#13343b" />
      <path d="M300 330 L512 720 L724 330" fill="none" stroke="#f3ead8" strokeWidth="84" strokeLinecap="round" strokeLinejoin="round" />
      <circle cx="300" cy="330" r="74" fill="#f3ead8" />
      <circle cx="724" cy="330" r="74" fill="#f3ead8" />
      <circle cx="512" cy="720" r="86" fill="#e9a23b" />
    </svg>
  );
}
