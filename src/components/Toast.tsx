import { useEffect } from "react";
import { CheckCircle, Info, Warning, X, XCircle } from "@phosphor-icons/react";
import styles from "./Toast.module.css";

export interface ToastProps { message: string; type?: "info" | "success" | "warning" | "danger"; onClose: () => void; duration?: number; }
export function Toast({ message, type = "info", onClose, duration = 4000 }: ToastProps) {
  useEffect(() => { if (!duration) return; const timer = window.setTimeout(onClose, duration); return () => window.clearTimeout(timer); }, [duration, onClose]);
  const icon = type === "success" ? <CheckCircle weight="fill" /> : type === "warning" ? <Warning weight="fill" /> : type === "danger" ? <XCircle weight="fill" /> : <Info weight="fill" />;
  return <div className={`${styles.toast} ${styles[type]}`} role="alert"><span className={styles.icon}>{icon}</span><span className={styles.message}>{message}</span><button className={styles.closeButton} onClick={onClose} aria-label="Close notification"><X /></button></div>;
}
