import { describe, expect, it } from "vitest";
import { pushSpeedSample } from "./speedHistory";

describe("pushSpeedSample", () => {
  it("appends samples in order", () => {
    expect(pushSpeedSample([1, 2], 3)).toEqual([1, 2, 3]);
  });

  it("does not mutate the input history", () => {
    const history = [1, 2];
    pushSpeedSample(history, 3);
    expect(history).toEqual([1, 2]);
  });

  it("clamps negative and non-finite speeds to zero", () => {
    expect(pushSpeedSample([], -5)).toEqual([0]);
    expect(pushSpeedSample([], Number.NaN)).toEqual([0]);
    expect(pushSpeedSample([], Number.POSITIVE_INFINITY)).toEqual([0]);
  });

  it("keeps only the most recent samples", () => {
    expect(pushSpeedSample([1, 2, 3], 4, 3)).toEqual([2, 3, 4]);
  });

  it("keeps every sample while under the limit", () => {
    expect(pushSpeedSample([1, 2], 3, 5)).toEqual([1, 2, 3]);
  });

  it("keeps exactly the limit", () => {
    expect(pushSpeedSample([1, 2, 3], 4, 4)).toEqual([1, 2, 3, 4]);
  });
});
