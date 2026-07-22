import styles from "./MorphingInfinity.module.css";

interface MorphingInfinityProps {
  label?: string;
  className?: string;
  size?: number;
}

export function MorphingInfinity({
  label = "Thinking…",
  className = "",
  size = 24,
}: MorphingInfinityProps) {
  return (
    <span className={`${styles.container} ${className}`} aria-label={label || "Thinking"}>
      <svg
        width={size}
        height={Math.round(size / 2)}
        viewBox="0 0 48 24"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
        className={styles.svg}
      >
        <defs>
          <linearGradient id="morphingInfinityGrad" x1="0%" y1="0%" x2="100%" y2="100%">
            <stop offset="0%" stopColor="var(--accent, #3b82f6)" />
            <stop offset="50%" stopColor="#a855f7" />
            <stop offset="100%" stopColor="#ec4899" />
          </linearGradient>
        </defs>

        {/* Glow backdrop track */}
        <path
          d="M 12 12 C 4 4, 4 20, 12 12 C 20 4, 28 4, 36 12 C 44 20, 44 4, 36 12 C 28 20, 20 20, 12 12 Z"
          className={styles.glowTrack}
        />

        {/* Morphing animated stroke */}
        <path
          d="M 12 12 C 4 4, 4 20, 12 12 C 20 4, 28 4, 36 12 C 44 20, 44 4, 36 12 C 28 20, 20 20, 12 12 Z"
          stroke="url(#morphingInfinityGrad)"
          className={styles.morphPath}
        />
      </svg>
      {label && <span className={styles.label}>{label}</span>}
    </span>
  );
}
