import { useState } from "react";
import { ChatCircleDots, ArrowUp } from "@phosphor-icons/react";
import styles from "./InteractiveChoiceBox.module.css";

export interface InteractiveChoiceBoxProps {
  question: string;
  options: string[];
  onSubmit: (answer: string) => void;
  onDismiss?: () => void;
}

export function InteractiveChoiceBox({ question, options, onSubmit, onDismiss }: InteractiveChoiceBoxProps) {
  const [selectedIndex, setSelectedIndex] = useState<number | null>(null);
  const [customText, setCustomText] = useState("");
  const isCustom = selectedIndex === -1;

  const handleSubmit = () => {
    if (isCustom && customText.trim()) {
      onSubmit(customText.trim());
    } else if (selectedIndex !== null && selectedIndex >= 0 && selectedIndex < options.length) {
      onSubmit(options[selectedIndex]);
    }
  };

  const canSubmit = (selectedIndex !== null && selectedIndex >= 0) || (isCustom && customText.trim().length > 0);

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
            key={index}
            type="button"
            className={`${styles.choiceOption} ${selectedIndex === index ? styles.choiceOptionSelected : ""}`}
            onClick={() => setSelectedIndex(index)}
          >
            <div className={`${styles.choiceRadio} ${selectedIndex === index ? styles.choiceRadioSelected : ""}`}>
              {selectedIndex === index && <div className={styles.choiceRadioDot} />}
            </div>
            <span>{option}</span>
          </button>
        ))}

        {/* Custom write-in option */}
        <button
          type="button"
          className={`${styles.choiceOption} ${isCustom ? styles.choiceOptionSelected : ""}`}
          onClick={() => setSelectedIndex(-1)}
        >
          <div className={`${styles.choiceRadio} ${isCustom ? styles.choiceRadioSelected : ""}`}>
            {isCustom && <div className={styles.choiceRadioDot} />}
          </div>
          <span>Custom response…</span>
        </button>

        {isCustom && (
          <textarea
            className={styles.choiceCustomInput}
            placeholder="Type your custom response here…"
            value={customText}
            onChange={(e) => setCustomText(e.target.value)}
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
        <button
          type="button"
          className={styles.choiceSubmit}
          disabled={!canSubmit}
          onClick={handleSubmit}
        >
          <ArrowUp size={12} weight="bold" />
          Submit
        </button>
      </div>
    </div>
  );
}
