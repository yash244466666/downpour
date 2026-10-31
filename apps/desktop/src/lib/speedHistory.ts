import { useEffect, useRef, useState } from "react";

/// Rolling window for the Total speed sparkline: 60 samples at the 1.2 s poll
/// cadence is roughly the last 72 seconds.
export const SPEED_HISTORY_LIMIT = 60;
export const SPEED_SAMPLE_INTERVAL_MS = 1200;

/**
 * Append one aggregate-speed sample, clamping negative/non-finite values to
 * zero and keeping only the most recent `limit` samples.
 */
export function pushSpeedSample(
  history: number[],
  speed: number,
  limit = SPEED_HISTORY_LIMIT,
): number[] {
  const sample = Number.isFinite(speed) && speed > 0 ? speed : 0;
  const next = [...history, sample];
  return next.length > limit ? next.slice(next.length - limit) : next;
}

/**
 * Sample the current aggregate speed on a fixed timer so a constant speed still
 * produces a continuous graph. The first render seeds the line with the current
 * value instead of an empty chart.
 */
export function useSpeedHistory(
  speed: number,
  intervalMs = SPEED_SAMPLE_INTERVAL_MS,
  limit = SPEED_HISTORY_LIMIT,
): number[] {
  // Two identical points keep a flat line visible before the first tick.
  const [history, setHistory] = useState<number[]>(() => [speed, speed]);
  const speedRef = useRef(speed);

  useEffect(() => {
    speedRef.current = speed;
  }, [speed]);

  useEffect(() => {
    const timer = window.setInterval(() => {
      setHistory((current) => pushSpeedSample(current, speedRef.current, limit));
    }, intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs, limit]);

  return history;
}
