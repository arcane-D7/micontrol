/**
 * AiFeedback — thumbs up/down feedback buttons for AI analysis results.
 *
 * Stores feedback in localStorage under key "mipc_ai_feedback" as an array of
 * { analysisId, rating, timestamp } objects.
 */

import { useState, useEffect } from 'react';
import { t } from '../hooks/useI18n';
import './AiFeedback.css';

const FEEDBACK_KEY = 'mipc_ai_feedback';

export interface FeedbackEntry {
  analysisId: string;
  rating: 'up' | 'down';
  timestamp: string;
}

function loadFeedback(): FeedbackEntry[] {
  try {
    const raw = localStorage.getItem(FEEDBACK_KEY);
    return raw ? (JSON.parse(raw) as FeedbackEntry[]) : [];
  } catch {
    return [];
  }
}

function saveFeedback(entries: FeedbackEntry[]): void {
  try {
    localStorage.setItem(FEEDBACK_KEY, JSON.stringify(entries));
  } catch {
    /* ignore */
  }
}

interface Props {
  analysisId: string;
}

export default function AiFeedback({ analysisId }: Props) {
  const [feedback, setFeedback] = useState<FeedbackEntry | null>(null);
  const [showThanks, setShowThanks] = useState(false);

  // Load existing feedback for this analysis when the ID changes
  useEffect(() => {
    const existing = loadFeedback().find((e) => e.analysisId === analysisId);
    setFeedback(existing ?? null);
    setShowThanks(false);
  }, [analysisId]);

  function handleFeedback(rating: 'up' | 'down') {
    // Don't re-save if the same rating is clicked again
    if (feedback?.rating === rating) return;

    const entry: FeedbackEntry = {
      analysisId,
      rating,
      timestamp: new Date().toISOString(),
    };

    const existing = loadFeedback();
    const filtered = existing.filter((e) => e.analysisId !== analysisId);
    filtered.push(entry);
    saveFeedback(filtered);
    setFeedback(entry);
    setShowThanks(true);
  }

  return (
    <div className="ai-feedback">
      <span className="ai-feedback__label">{t('aiAnalysis.analysis.feedbackQuestion')}</span>
      <div className="ai-feedback__buttons">
        <button
          className={`ai-feedback__btn${feedback?.rating === 'up' ? ' ai-feedback__btn--active' : ''}`}
          onClick={() => handleFeedback('up')}
          aria-label={t('aiAnalysis.analysis.feedbackUp')}
          aria-pressed={feedback?.rating === 'up'}
          title={t('aiAnalysis.analysis.feedbackUp')}
        >
          <Icon name="thumbs-up" size={16} />
        </button>
        <button
          className={`ai-feedback__btn${feedback?.rating === 'down' ? ' ai-feedback__btn--active' : ''}`}
          onClick={() => handleFeedback('down')}
          aria-label={t('aiAnalysis.analysis.feedbackDown')}
          aria-pressed={feedback?.rating === 'down'}
          title={t('aiAnalysis.analysis.feedbackDown')}
        >
          <Icon name="thumbs-down" size={16} />
        </button>
      </div>
      {showThanks && (
        <span className="ai-feedback__thanks">{t('aiAnalysis.analysis.feedbackThanks')}</span>
      )}
    </div>
  );
}

import { Icon } from './ui/icons';
