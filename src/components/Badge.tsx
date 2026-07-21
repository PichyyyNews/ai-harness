import type { HTMLAttributes, ReactNode } from "react";
import styles from "./Badge.module.css";

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> { variant?: "default" | "success" | "warning" | "danger"; size?: "sm" | "md"; children: ReactNode; }
export function Badge({ children, className = "", variant = "default", size = "md", ...props }: BadgeProps) { return <span className={`${styles.badge} ${styles[variant]} ${styles[size]} ${className}`} {...props}>{children}</span>; }
