import type { CSSProperties } from "react";
import styles from "./TextShimmerWave.module.css";

interface TextShimmerWaveProps {
  text?: string;
  className?: string;
}

export function TextShimmerWave({ text = "Thinking...", className = "" }: TextShimmerWaveProps) {
  const characters = Array.from(text);

  return (
    <span className={`${styles.shimmerWrapper} ${className}`} aria-label={text}>
      {characters.map((char, index) => (
        <span
          key={`${char}-${index}`}
          className={styles.shimmerChar}
          style={{ "--char-index": index } as CSSProperties}
        >
          {char === " " ? "\u00A0" : char}
        </span>
      ))}
    </span>
  );
}
