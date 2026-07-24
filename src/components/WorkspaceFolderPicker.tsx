import React, { useState } from 'react';
import { Folder, FolderOpen, Check, Wrench } from '@phosphor-icons/react';
import styles from './WorkspaceFolderPicker.module.css';

interface WorkspaceFolderPickerProps {
  currentWorkspace: string;
  onSelectWorkspace: (path: string) => void;
  isAiderMode: boolean;
  onToggleAiderMode: (enabled: boolean) => void;
}

export const WorkspaceFolderPicker: React.FC<WorkspaceFolderPickerProps> = ({
  currentWorkspace,
  onSelectWorkspace,
  isAiderMode,
  onToggleAiderMode,
}) => {
  const [isEditing, setIsEditing] = useState(false);
  const [inputPath, setInputPath] = useState(currentWorkspace);

  const handleSave = () => {
    if (inputPath.trim()) {
      onSelectWorkspace(inputPath.trim());
    }
    setIsEditing(false);
  };

  return (
    <div className={styles.container}>
      <button
        className={`${styles.modeToggle} ${isAiderMode ? styles.active : ''}`}
        onClick={() => onToggleAiderMode(!isAiderMode)}
        title="Toggle Aider AI Pair Programming Backend"
      >
        <Wrench size={16} weight={isAiderMode ? 'fill' : 'regular'} />
        <span>Aider Mode</span>
      </button>

      <div className={styles.folderBadge}>
        <Folder size={16} className={styles.icon} />
        {isEditing ? (
          <div className={styles.inputGroup}>
            <input
              type="text"
              value={inputPath}
              onChange={(e) => setInputPath(e.target.value)}
              placeholder="C:\Users\...\your-project"
              className={styles.pathInput}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === 'Enter') handleSave();
                if (e.key === 'Escape') setIsEditing(false);
              }}
            />
            <button onClick={handleSave} className={styles.saveBtn}>
              <Check size={14} />
            </button>
          </div>
        ) : (
          <span
            className={styles.workspaceText}
            onClick={() => {
              setInputPath(currentWorkspace);
              setIsEditing(true);
            }}
            title="Click to set workspace project folder"
          >
            {currentWorkspace ? currentWorkspace : 'Select Project Folder'}
          </span>
        )}

        <button
          className={styles.changeBtn}
          onClick={() => {
            setInputPath(currentWorkspace);
            setIsEditing(!isEditing);
          }}
          title="Change Workspace Directory"
        >
          <FolderOpen size={14} />
        </button>
      </div>
    </div>
  );
};
