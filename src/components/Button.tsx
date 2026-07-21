import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Skeleton } from "./Skeleton";
import styles from "./Button.module.css";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "danger" | "ghost";
  size?: "sm" | "md" | "lg";
  iconPrefix?: ReactNode;
  iconSuffix?: ReactNode;
  fullWidth?: boolean;
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { children, className = "", variant = "primary", size = "md", iconPrefix, iconSuffix, fullWidth = false, type = "button", loading, ...props },
  ref,
) {
  if (loading) return <Skeleton variant="rect" className={className} style={{ width: fullWidth ? "100%" : "120px", height: { sm: "32px", md: "40px", lg: "48px" }[size] }} />;
  return (
    <button ref={ref} type={type} className={`${styles.btn} ${styles[variant]} ${styles[size]} ${fullWidth ? styles.fullWidth : ""} ${className}`} {...props}>
      {iconPrefix && <span className={styles.icon}>{iconPrefix}</span>}
      {children && <span className={styles.content}>{children}</span>}
      {iconSuffix && <span className={styles.icon}>{iconSuffix}</span>}
    </button>
  );
});
