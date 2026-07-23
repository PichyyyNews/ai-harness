import { useState } from "react";
import { ChatCircleDots, ArrowUp } from "@phosphor-icons/react";
import styles from "./InteractiveChoiceBox.module.css";

export interface ChoiceOption { id: string; label: string; }

export interface InteractiveChoiceBoxProps {
  question: string;
  options: ChoiceOption[];
  disabled?: boolean;
  allowCustom?: boolean;
  onSubmit: (optionId: string | null, answer: string) => void;
  onDismiss?: () => void;
}

export function InteractiveChoiceBox({ question, options, disabled = false, allowCustom = false, onSubmit, onDismiss }: InteractiveChoiceBoxProps) {
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [customText, setCustomText] = useState("");
  const isCustom = selectedIndex === -1;

  const handleOptionClick = (option: ChoiceOption, index: number) => {
    setSelectedIndex(index);
    // Instant auto-submit like Claude Q&A!
    onSubmit(option.id, option.label);
  };

  const handleCustomSubmit = () => {
    if (customText.trim()) {
      onSubmit(null, customText.trim());
    }
  };

  return (
    <div className={styles.choiceOverlay}>
      <div className={styles.choiceHeader}>
        <div className={styles.choiceIcon}>
          <ChatCircleDots size={16} weight="bold" />
        </div>
        <div className={styles.choiceQuestion}>{question}</div>
      </div>

      <div className={styles.choiceOptions}>
        {options.map((option, index) => (
          <button
            key={option.id}
            type="button"
            className={`${styles.choiceOption} ${selectedIndex === index ? styles.choiceOptionSelected : ""}`}
            onClick={() => handleOptionClick(option, index)}
            disabled={disabled}
          >
            <div className={`${styles.choiceRadio} ${selectedIndex === index ? styles.choiceRadioSelected : ""}`}>
              {selectedIndex === index && <div className={styles.choiceRadioDot} />}
            </div>
            <span>{option.label}</span>
          </button>
        ))}

        {/* Custom write-in option */}
        {allowCustom && <button
          type="button"
          className={`${styles.choiceOption} ${isCustom ? styles.choiceOptionSelected : ""}`}
          onClick={() => setSelectedIndex(-1)}
          disabled={disabled}
        >
          <div className={`${styles.choiceRadio} ${isCustom ? styles.choiceRadioSelected : ""}`}>
            {isCustom && <div className={styles.choiceRadioDot} />}
          </div>
          <span>Custom response…</span>
        </button>}

        {allowCustom && isCustom && (
          <textarea
            className={styles.choiceCustomInput}
            placeholder="Type your custom response here…"
            value={customText}
            onChange={(e) => setCustomText(e.target.value)}
            disabled={disabled}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                handleCustomSubmit();
              }
            }}
            rows={2}
            autoFocus
          />
        )}
      </div>

      <div className={styles.choiceFooter}>
        {onDismiss && (
          <button type="button" className={styles.choiceDismiss} onClick={onDismiss}>
            Skip
          </button>
        )}
        {isCustom && (
          <button
            type="button"
            className={styles.choiceSubmit}
            disabled={disabled || !customText.trim()}
            onClick={handleCustomSubmit}
          >
            <ArrowUp size={12} weight="bold" />
            Submit
          </button>
        )}
      </div>
    </div>
  );
}
