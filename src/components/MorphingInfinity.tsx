import styles from "./MorphingInfinity.module.css";

interface MorphingInfinityProps {
  label?: string;
  className?: string;
}

export function MorphingInfinity({ label = "Thinking…", className = "" }: MorphingInfinityProps) {
  return (
    <span className={`${styles.infinityWrapper} ${className}`} aria-label={label}>
      <svg
        className={styles.infinitySvg}
        viewBox="0 0 36 18"
        fill="none"
        xmlns="http://www.w3.org/2000/svg"
      >
        <defs>
          <linearGradient id="infinityGradient" x1="0%" y1="0%" x2="100%" y2="0%">
            <stop offset="0%" stopColor="var(--accent, #3b82f6)" />
            <stop offset="50%" stopColor="#8b5cf6" />
            <stop offset="100%" stopColor="var(--accent, #3b82f6)" />
          </linearGradient>
        </defs>

        {/* Background track */}
        <path
          d="M 9,9 C 3,3 3,15 9,9 C 15,3 21,3 27,9 C 33,15 33,3 27,9 C 21,15 15,15 9,9 Z"
          className={styles.infinityPathBg}
        />

        {/* Morphing animated stroke */}
        <path
          d="M 9,9 C 3,3 3,15 9,9 C 15,3 21,3 27,9 C 33,15 33,3 27,9 C 21,15 15,15 9,9 Z"
          stroke="url(#infinityGradient)"
          className={styles.infinityPath}
        />
      </svg>

      {label && <span className={styles.thinkingLabel}>{label}</span>}
    </span>
  );
}
