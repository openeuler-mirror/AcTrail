"""Distribution-level statistics for performance reports."""

from __future__ import annotations

import itertools
import math
import random
import statistics as stats
from dataclasses import dataclass


@dataclass(frozen=True)
class DistributionStats:
    hl_overhead_percent: float
    ci_low_percent: float
    ci_high_percent: float
    ks_p_value: float
    mw_p_value: float
    decision: str


def distribution_stats(
    baseline: list[float],
    observed: list[float],
    overhead_threshold_percent: float,
    alpha: float,
    bootstrap_resamples: int,
    permutation_resamples: int,
    max_exact_permutations: int,
    rng: random.Random,
) -> DistributionStats:
    threshold = overhead_threshold_percent / 100.0
    ratio = hodges_lehmann_ratio(baseline, observed)
    ci_low, ci_high = bootstrap_hl_ratio_ci(
        baseline,
        observed,
        alpha,
        bootstrap_resamples,
        rng,
    )
    ks_p_value = ks_same_distribution_p_value(
        baseline,
        observed,
        permutation_resamples,
        max_exact_permutations,
        rng,
    )
    mw_p_value = mann_whitney_greater_p_value(
        [value * (1.0 + threshold) for value in baseline],
        observed,
        permutation_resamples,
        max_exact_permutations,
        rng,
    )
    decision = distribution_decision(ci_low, ci_high, threshold, alpha, mw_p_value)
    return DistributionStats(
        hl_overhead_percent=(ratio - 1.0) * 100.0,
        ci_low_percent=(ci_low - 1.0) * 100.0,
        ci_high_percent=(ci_high - 1.0) * 100.0,
        ks_p_value=ks_p_value,
        mw_p_value=mw_p_value,
        decision=decision,
    )


def distribution_decision(
    ci_low: float,
    ci_high: float,
    threshold: float,
    alpha: float,
    mw_p_value: float,
) -> str:
    threshold_ratio = 1.0 + threshold
    if ci_high <= threshold_ratio:
        return "CI supports <= threshold"
    if ci_low > threshold_ratio and mw_p_value <= alpha:
        return "CI and rank test support > threshold"
    if ci_low > threshold_ratio:
        return "CI supports > threshold; rank test inconclusive"
    if mw_p_value <= alpha:
        return "rank test supports > threshold; CI overlaps"
    return "inconclusive"


def hodges_lehmann_ratio(baseline: list[float], observed: list[float]) -> float:
    ratios = [observed_value / baseline_value for observed_value in observed for baseline_value in baseline]
    return stats.median(ratios)


def bootstrap_hl_ratio_ci(
    baseline: list[float],
    observed: list[float],
    alpha: float,
    resamples: int,
    rng: random.Random,
) -> tuple[float, float]:
    values = []
    for _ in range(resamples):
        baseline_sample = [rng.choice(baseline) for _ in baseline]
        observed_sample = [rng.choice(observed) for _ in observed]
        values.append(hodges_lehmann_ratio(baseline_sample, observed_sample))
    values.sort()
    return (
        quantile_sorted(values, alpha / 2.0),
        quantile_sorted(values, 1.0 - alpha / 2.0),
    )


def ks_same_distribution_p_value(
    baseline: list[float],
    observed: list[float],
    resamples: int,
    max_exact_permutations: int,
    rng: random.Random,
) -> float:
    observed_stat = ks_statistic(baseline, observed)
    pooled = baseline + observed
    baseline_size = len(baseline)
    total = math.comb(len(pooled), baseline_size)
    if total <= max_exact_permutations:
        at_least = 0
        for indexes in itertools.combinations(range(len(pooled)), baseline_size):
            baseline_indexes = set(indexes)
            permuted_baseline = [pooled[index] for index in baseline_indexes]
            permuted_observed = [
                value for index, value in enumerate(pooled) if index not in baseline_indexes
            ]
            if ks_statistic(permuted_baseline, permuted_observed) >= observed_stat:
                at_least += 1
        return at_least / total
    at_least = 0
    for _ in range(resamples):
        sample = pooled[:]
        rng.shuffle(sample)
        permuted_baseline = sample[:baseline_size]
        permuted_observed = sample[baseline_size:]
        if ks_statistic(permuted_baseline, permuted_observed) >= observed_stat:
            at_least += 1
    return at_least / resamples


def ks_statistic(first: list[float], second: list[float]) -> float:
    first_sorted = sorted(first)
    second_sorted = sorted(second)
    thresholds = sorted(set(first_sorted + second_sorted))
    result = 0.0
    first_index = 0
    second_index = 0
    for threshold in thresholds:
        while first_index < len(first_sorted) and first_sorted[first_index] <= threshold:
            first_index += 1
        while second_index < len(second_sorted) and second_sorted[second_index] <= threshold:
            second_index += 1
        first_cdf = first_index / len(first_sorted)
        second_cdf = second_index / len(second_sorted)
        result = max(result, abs(first_cdf - second_cdf))
    return result


def mann_whitney_greater_p_value(
    baseline_shifted: list[float],
    observed: list[float],
    resamples: int,
    max_exact_permutations: int,
    rng: random.Random,
) -> float:
    observed_stat = mann_whitney_u(observed, baseline_shifted)
    pooled = observed + baseline_shifted
    observed_size = len(observed)
    ranks = midranks(pooled)
    total = math.comb(len(pooled), observed_size)
    if total <= max_exact_permutations:
        at_least = 0
        for indexes in itertools.combinations(range(len(pooled)), observed_size):
            if u_from_rank_indexes(ranks, indexes, observed_size) >= observed_stat:
                at_least += 1
        return at_least / total
    at_least = 0
    for _ in range(resamples):
        indexes = rng.sample(range(len(pooled)), observed_size)
        if u_from_rank_indexes(ranks, indexes, observed_size) >= observed_stat:
            at_least += 1
    return at_least / resamples


def mann_whitney_u(first: list[float], second: list[float]) -> float:
    ranks = midranks(first + second)
    return u_from_rank_indexes(ranks, range(len(first)), len(first))


def midranks(values: list[float]) -> list[float]:
    ordered = sorted(enumerate(values), key=lambda item: item[1])
    ranks = [0.0] * len(values)
    position = 0
    while position < len(ordered):
        end = position + 1
        while end < len(ordered) and ordered[end][1] == ordered[position][1]:
            end += 1
        rank = (position + 1 + end) / 2.0
        for index in range(position, end):
            ranks[ordered[index][0]] = rank
        position = end
    return ranks


def u_from_rank_indexes(ranks: list[float], indexes, sample_size: int) -> float:
    rank_sum = sum(ranks[index] for index in indexes)
    return rank_sum - sample_size * (sample_size + 1) / 2.0


def quantile_sorted(values: list[float], probability: float) -> float:
    if len(values) == 1:
        return values[0]
    rank = (len(values) - 1) * probability
    lower = int(rank)
    upper = min(lower + 1, len(values) - 1)
    fraction = rank - lower
    return values[lower] + (values[upper] - values[lower]) * fraction
