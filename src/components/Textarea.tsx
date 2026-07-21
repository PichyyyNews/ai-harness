import { forwardRef, useId, type TextareaHTMLAttributes } from "react";
import styles from "./Textarea.module.css";

export interface TextareaProps extends TextareaHTMLAttributes<HTMLTextAreaElement> { label?: string; error?: string; description?: string; }
export const Textarea = forwardRef<HTMLTextAreaElement, TextareaProps>(function Textarea({ className = "", label, error, description, id, ...props }, ref) {
  const textareaId = id ?? useId();
  return <div className={`${styles.wrapper} ${className}`}>
    {label && <label htmlFor={textareaId} className={styles.label}>{label}</label>}
    <textarea ref={ref} id={textareaId} className={`${styles.textarea} ${error ? styles.hasError : ""}`} {...props} />
    {error ? <p className={styles.error}>{error}</p> : description && <p className={styles.description}>{description}</p>}
  </div>;
});
