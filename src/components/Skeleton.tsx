import type { CSSProperties } from "react";
import styles from "./Skeleton.module.css";

export interface SkeletonProps {
  variant?: "text" | "circle" | "rect";
  width?: string | number;
  height?: string | number;
  className?: string;
  style?: CSSProperties;
}

export function Skeleton({ variant = "rect", width, height, className = "", style }: SkeletonProps) {
  return <div className={`${styles.skeleton} ${styles[variant]} ${className}`} style={{ width, height, ...style }} aria-hidden="true" />;
}
