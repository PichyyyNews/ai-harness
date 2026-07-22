import type { ComponentProps } from "react";
import { motion } from "motion/react";

const circleA =
  "M 12 8 C 14.21 8 16 9.79 16 12 C 16 14.21 14.21 16 12 16 C 9.79 16 8 14.21 8 12 C 8 9.79 9.79 8 12 8 Z";

const infinity =
  "M 12 12 C 14 8.5 19 8.5 19 12 C 19 15.5 14 15.5 12 12 C 10 8.5 5 8.5 5 12 C 5 15.5 10 15.5 12 12 Z";

const circleB =
  "M 12 16 C 14.21 16 16 14.21 16 12 C 16 9.79 14.21 8 12 8 C 9.79 8 8 9.79 8 12 C 8 14.21 9.79 16 12 16 Z";

export interface MorphingInfinityProps extends ComponentProps<"svg"> {
  label?: string;
}

export function MorphingInfinity({ label, className = "", style, ...props }: MorphingInfinityProps) {
  return (
    <span style={{ display: "inline-flex", alignItems: "center", gap: "7px", verticalAlign: "middle" }}>
      <svg
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.8}
        strokeLinecap="round"
        strokeLinejoin="round"
        role="status"
        aria-label="Loading"
        className={className}
        style={{ width: "20px", height: "20px", overflow: "visible", color: "var(--accent, currentColor)", ...style }}
        {...props}
      >
        <motion.path
          animate={{
            d: [circleA, infinity, circleB, infinity, circleA],
          }}
          transition={{
            d: {
              duration: 3.2,
              ease: "easeInOut",
              repeat: Infinity,
              times: [0, 0.25, 0.5, 0.75, 1.0],
            },
          }}
        />
      </svg>
      {label && <span style={{ fontSize: "12px", color: "var(--text-secondary)", fontWeight: 500 }}>{label}</span>}
    </span>
  );
}
