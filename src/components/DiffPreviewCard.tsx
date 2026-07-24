import React from 'react';
import { GitCommit, TerminalWindow, WarningCircle } from '@phosphor-icons/react';
import styles from './DiffPreviewCard.module.css';

interface DiffPreviewCardProps {
  logs: string[];
  isDone?: boolean;
  hasError?: boolean;
}

const decodeUnicode = (str: string) => {
  try {
    return str.replace(/\\u([0-9a-fA-F]{4})/g, (_, code) =>
      String.fromCharCode(parseInt(code, 16))
    );
  } catch {
    return str;
  }
};

export const DiffPreviewCard: React.FC<DiffPreviewCardProps> = ({ logs, isDone, hasError }) => {
  if (!logs || logs.length === 0) return null;

  return (
    <div className={styles.card}>
      <div className={styles.header}>
        <div className={styles.titleGroup}>
          <TerminalWindow size={16} className={styles.icon} />
          <span>Aider Execution Log</span>
        </div>
        {hasError ? (
          <div className={styles.badgeDone} style={{ backgroundColor: 'rgba(239, 68, 68, 0.15)', color: '#f87171', borderColor: 'rgba(239, 68, 68, 0.3)' }}>
            <WarningCircle size={14} />
            <span>Error</span>
          </div>
        ) : (
          isDone && (
            <div className={styles.badgeDone}>
              <GitCommit size={14} />
              <span>Committed</span>
            </div>
          )
        )}
      </div>

      <div className={styles.terminalBody}>
        {logs.map((rawLine, idx) => {
          const line = decodeUnicode(rawLine);
          const isDiffLineAdd = line.startsWith('+') && !line.startsWith('+++');
          const isDiffLineRemove = line.startsWith('-') && !line.startsWith('---');
          const isFileHeader = line.includes('Applied edit to') || line.includes('Commit') || line.includes('aider');
          const isErrorLine = line.includes('Error') || line.includes('Traceback') || line.includes('Failed');

          let lineClass = styles.logLine;
          if (isDiffLineAdd) lineClass = `${styles.logLine} ${styles.add}`;
          if (isDiffLineRemove) lineClass = `${styles.logLine} ${styles.remove}`;
          if (isFileHeader) lineClass = `${styles.logLine} ${styles.fileHeader}`;
          if (isErrorLine) lineClass = `${styles.logLine} ${styles.remove}`;

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
