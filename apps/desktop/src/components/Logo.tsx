export function Logo({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 40 40"
      className={className}
      role="img"
      aria-label="Downpour"
    >
      <defs>
        <linearGradient id="dp-logo-bg" x1="0" y1="0" x2="1" y2="1">
          <stop offset="0" stopColor="#0b1220" />
          <stop offset="1" stopColor="#0e7490" />
        </linearGradient>
        <linearGradient id="dp-logo-drop" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0" stopColor="#7dd3fc" />
          <stop offset="1" stopColor="#22d3ee" />
        </linearGradient>
      </defs>
      <rect width="40" height="40" rx="11" fill="url(#dp-logo-bg)" />
      <path
        d="M20 8c2.9 4.6 7.2 9.2 7.2 13.9a7.2 7.2 0 1 1-14.4 0C12.8 17.2 17.1 12.6 20 8z"
        fill="url(#dp-logo-drop)"
      />
      <path
        d="M20 16v9.4"
        stroke="#071019"
        strokeWidth="2.4"
        strokeLinecap="round"
      />
      <path
        d="M16.2 22.1 20 25.9l3.8-3.8"
        fill="none"
        stroke="#071019"
        strokeWidth="2.4"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
