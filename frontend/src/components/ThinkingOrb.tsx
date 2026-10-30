'use client';

/**
 * Activity indicator drawn from the Voga mark: the three points of the V light up in turn.
 * `thinking`: an AI answer; `background`: a long job (collecting, indexing); `working`: quick
 * loading or saving. Prefer it over a spinning icon for these states.
 */
export type OrbVariant = 'thinking' | 'background' | 'working';

const SPEED: Record<OrbVariant, string> = { thinking: '1.2s', background: '2.4s', working: '0.9s' };
const POINTS = [
  [300, 330],
  [512, 720],
  [724, 330],
];

export function ThinkingOrb({
  variant = 'thinking',
  size = 24,
  label,
  className = '',
}: {
  variant?: OrbVariant;
  size?: number;
  label?: string;
  className?: string;
}) {
  const duration = SPEED[variant];
  return (
    <svg
      width={size}
      height={size}
      viewBox="200 230 624 590"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
      className={`inline-block shrink-0 ${className}`}
    >
      <path d="M300 330 L512 720 L724 330" fill="none" stroke="currentColor" strokeOpacity="0.25" strokeWidth="70" strokeLinecap="round" strokeLinejoin="round" />
      {POINTS.map(([cx, cy], i) => (
        <circle key={i} cx={cx} cy={cy} r="90" fill={i === 1 ? '#e9a23b' : 'currentColor'}>
          <animate
            attributeName="opacity"
            values="0.25;1;0.25"
            dur={duration}
            begin={`${(i * parseFloat(duration)) / 3}s`}
            repeatCount="indefinite"
          />
        </circle>
      ))}
    </svg>
  );
}
