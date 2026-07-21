import type { HTMLAttributes, ReactNode } from "react";
import { Skeleton } from "./Skeleton";
import styles from "./Card.module.css";

interface CardProps extends HTMLAttributes<HTMLDivElement> { children: ReactNode; loading?: boolean; }
interface CardSectionProps extends HTMLAttributes<HTMLDivElement> { children: ReactNode; }

function CardBase({ children, className = "", loading, ...props }: CardProps) {
  if (loading) return <div className={`${styles.card} ${className}`} {...props}><Skeleton variant="text" width="40%" height="18px" /><Skeleton variant="text" width="85%" height="14px" /></div>;
  return <div className={`${styles.card} ${className}`} {...props}>{children}</div>;
}

function CardHeader({ children, className = "", ...props }: CardSectionProps) { return <div className={`${styles.header} ${className}`} {...props}>{children}</div>; }
function CardBody({ children, className = "", ...props }: CardSectionProps) { return <div className={`${styles.body} ${className}`} {...props}>{children}</div>; }
function CardFooter({ children, className = "", ...props }: CardSectionProps) { return <div className={`${styles.footer} ${className}`} {...props}>{children}</div>; }

export const Card = Object.assign(CardBase, { Header: CardHeader, Body: CardBody, Footer: CardFooter });
