import React from 'react';
import { GitCommit, Code, TerminalWindow } from '@phosphor-icons/react';
import styles from './DiffPreviewCard.module.css';

interface DiffPreviewCardProps {
  logs: string[];
  isDone?: boolean;
}

export const DiffPreviewCard: React.FC<DiffPreviewCardProps> = ({ logs, isDone }) => {
  if (!logs || logs.length === 0) return null;

  return (
    <div className={styles.card}>
      <div className={styles.header}>
        <div className={styles.titleGroup}>
          <TerminalWindow size={16} className={styles.icon} />
          <span>Aider Execution Log</span>
        </div>
        {isDone && (
          <div className={styles.badgeDone}>
            <GitCommit size={14} />
            <span>Committed</span>
          </div>
        )}
      </div>

      <div className={styles.terminalBody}>
        {logs.map((line, idx) => {
          const isDiffLineAdd = line.startsWith('+') && !line.startsWith('+++');
          const isDiffLineRemove = line.startsWith('-') && !line.startsWith('---');
          const isFileHeader = line.includes('Applied edit to') || line.includes('Commit');

          let lineClass = styles.logLine;
          if (isDiffLineAdd) lineClass = `${styles.logLine} ${styles.add}`;
          if (isDiffLineRemove) lineClass = `${styles.logLine} ${styles.remove}`;
          if (isFileHeader) lineClass = `${styles.logLine} ${styles.fileHeader}`;

          return (
            <div key={idx} className={lineClass}>
              {line}
            </div>
          );
        })}
      </div>
    </div>
  );
};
