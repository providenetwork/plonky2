use itertools::{izip, multiunzip};
use plonky2::field::extension_field::Extendable;
use plonky2::field::field_types::{Field, PrimeField64};
use plonky2::field::packed_field::PackedField;
use plonky2::hash::hash_types::RichField;
use plonky2::plonk::circuit_builder::CircuitBuilder;
use starky::constraint_consumer::{ConstraintConsumer, RecursiveConstraintConsumer};
use starky::vars::{StarkEvaluationTargets, StarkEvaluationVars};

use crate::public_input_layout::NUM_PUBLIC_INPUTS;
use crate::registers::memory::*;
use crate::registers::NUM_COLUMNS;

#[derive(Default)]
pub struct TransactionMemory {
    pub calls: Vec<ContractMemory>,
}

/// A virtual memory space specific to the current contract call.
pub struct ContractMemory {
    pub code: MemorySegment,
    pub main: MemorySegment,
    pub calldata: MemorySegment,
    pub returndata: MemorySegment,
}

pub struct MemorySegment {
    pub content: Vec<u8>,
}

pub(crate) fn generate_memory<F: PrimeField64>(trace_cols: &mut [Vec<F>]) {
    let context = &trace_cols[MEMORY_ADDR_CONTEXT];
    let segment = &trace_cols[MEMORY_ADDR_SEGMENT];
    let virtuals = &trace_cols[MEMORY_ADDR_VIRTUAL];
    let from = &trace_cols[MEMORY_FROM];
    let to = &trace_cols[MEMORY_TO];
    let timestamp = &trace_cols[MEMORY_TIMESTAMP];

    let (sorted_context, sorted_segment, sorted_virtual, sorted_from, sorted_to, sorted_timestamp) =
        sort_memory_ops(context, segment, virtuals, from, to, timestamp);

    let (trace_context, trace_segment, trace_virtual, two_traces_combined, all_traces_combined) =
        generate_traces(context, segment, virtuals, from, to, timestamp);

    trace_cols[SORTED_MEMORY_ADDR_CONTEXT] = sorted_context;
    trace_cols[SORTED_MEMORY_ADDR_SEGMENT] = sorted_segment;
    trace_cols[SORTED_MEMORY_ADDR_VIRTUAL] = sorted_virtual;
    trace_cols[SORTED_MEMORY_FROM] = sorted_from;
    trace_cols[SORTED_MEMORY_TO] = sorted_to;
    trace_cols[SORTED_MEMORY_TIMESTAMP] = sorted_timestamp;

    trace_cols[MEMORY_TRACE_CONTEXT] = trace_context;
    trace_cols[MEMORY_TRACE_SEGMENT] = trace_segment;
    trace_cols[MEMORY_TRACE_VIRTUAL] = trace_virtual;
    trace_cols[MEMORY_TWO_TRACES_COMBINED] = two_traces_combined;
    trace_cols[MEMORY_ALL_TRACES_COMBINED] = all_traces_combined;
}

pub fn sort_memory_ops<F: PrimeField64>(
    context: &[F],
    segment: &[F],
    virtuals: &[F],
    from: &[F],
    to: &[F],
    timestamp: &[F],
) -> (Vec<F>, Vec<F>, Vec<F>, Vec<F>, Vec<F>, Vec<F>) {
    let mut ops: Vec<(F, F, F, F, F, F)> = izip!(
        context.iter().cloned(),
        segment.iter().cloned(),
        virtuals.iter().cloned(),
        from.iter().cloned(),
        to.iter().cloned(),
        timestamp.iter().cloned()
    )
    .collect();

    ops.sort_by(|&(c1, s1, v1, _, _, t1), &(c2, s2, v2, _, _, t2)| {
        (
            c1.to_noncanonical_u64(),
            s1.to_noncanonical_u64(),
            v1.to_noncanonical_u64(),
            t1.to_noncanonical_u64(),
        )
            .cmp(&(
                c2.to_noncanonical_u64(),
                s2.to_noncanonical_u64(),
                v2.to_noncanonical_u64(),
                t2.to_noncanonical_u64(),
            ))
    });

    multiunzip(ops)
}

pub fn generate_traces<F: PrimeField64>(
    context: &[F],
    segment: &[F],
    virtuals: &[F],
    from: &[F],
    to: &[F],
    timestamp: &[F],
) -> (Vec<F>, Vec<F>, Vec<F>, Vec<F>, Vec<F>) {
    let num_ops = context.len();
    let mut trace_context = Vec::new();
    let mut trace_segment = Vec::new();
    let mut trace_virtual = Vec::new();
    let mut two_traces_combined = Vec::new();
    let mut all_traces_combined = Vec::new();
    for idx in 0..num_ops - 1 {
        let this_trace_context = if context[idx] == context[idx + 1] {
            F::ONE
        } else {
            F::ZERO
        };
        let this_trace_segment = if segment[idx] == segment[idx + 1] {
            F::ONE
        } else {
            F::ZERO
        };
        let this_trace_virtual = if virtuals[idx] == virtuals[idx + 1] {
            F::ONE
        } else {
            F::ZERO
        };

        trace_context.push(this_trace_context);
        trace_segment.push(this_trace_segment);
        trace_virtual.push(this_trace_virtual);

        two_traces_combined.push((F::ONE - this_trace_context) * (F::ONE - this_trace_segment));
        all_traces_combined.push(
            (F::ONE - this_trace_context)
                * (F::ONE - this_trace_segment)
                * (F::ONE - this_trace_virtual),
        );
    }

    trace_context.push(F::ZERO);
    trace_segment.push(F::ZERO);
    trace_virtual.push(F::ZERO);
    two_traces_combined.push(F::ZERO);
    all_traces_combined.push(F::ZERO);

    (
        trace_context,
        trace_segment,
        trace_virtual,
        two_traces_combined,
        all_traces_combined,
    )
}

pub(crate) fn eval_memory<F: Field, P: PackedField<Scalar = F>>(
    vars: StarkEvaluationVars<F, P, NUM_COLUMNS, NUM_PUBLIC_INPUTS>,
    yield_constr: &mut ConstraintConsumer<P>,
) {
    let one = P::from(F::ONE);

    let addr_context = vars.local_values[SORTED_MEMORY_ADDR_CONTEXT];
    let addr_segment = vars.local_values[SORTED_MEMORY_ADDR_SEGMENT];
    let addr_virtual = vars.local_values[SORTED_MEMORY_ADDR_VIRTUAL];
    let from = vars.local_values[SORTED_MEMORY_FROM]; // TODO: replace "from" and "to" with "val" and "R/W"
    let to = vars.local_values[SORTED_MEMORY_TO];
    let timestamp = vars.local_values[SORTED_MEMORY_TIMESTAMP];

    let next_addr_context = vars.next_values[SORTED_MEMORY_ADDR_CONTEXT];
    let next_addr_segment = vars.next_values[SORTED_MEMORY_ADDR_SEGMENT];
    let next_addr_virtual = vars.next_values[SORTED_MEMORY_ADDR_VIRTUAL];
    let next_from = vars.next_values[SORTED_MEMORY_FROM];
    let next_to = vars.next_values[SORTED_MEMORY_TO];
    let next_timestamp = vars.next_values[SORTED_MEMORY_TIMESTAMP];

    let trace_context = vars.local_values[MEMORY_TRACE_CONTEXT];
    let trace_segment = vars.local_values[MEMORY_TRACE_SEGMENT];
    let trace_virtual = vars.local_values[MEMORY_TRACE_VIRTUAL];
    let two_traces_combined = vars.local_values[MEMORY_TWO_TRACES_COMBINED];
    let all_traces_combined = vars.local_values[MEMORY_ALL_TRACES_COMBINED];

    let not_trace_context = one - trace_context;
    let not_trace_segment = one - trace_segment;
    let not_trace_virtual = one - trace_virtual;

    // First set of ordering constraint: traces are boolean.
    yield_constr.constraint(trace_context * not_trace_context);
    yield_constr.constraint(trace_segment * not_trace_segment);
    yield_constr.constraint(trace_virtual * not_trace_virtual);

    // Second set of ordering constraints: trace matches with no change in corresponding column.
    yield_constr.constraint(trace_context * (next_addr_context - addr_context));
    yield_constr.constraint(trace_segment * (next_addr_segment - addr_segment));
    yield_constr.constraint(trace_virtual * (next_addr_virtual - addr_virtual));

    let context_range_check =
        vars.local_values[crate::registers::range_check_degree::col_rc_degree_input(0)];
    let segment_range_check =
        vars.local_values[crate::registers::range_check_degree::col_rc_degree_input(1)];
    let virtual_range_check =
        vars.local_values[crate::registers::range_check_degree::col_rc_degree_input(2)];

    // Third set of ordering constraints: range-check difference in the column that should be increasing.
    yield_constr.constraint(
        context_range_check
            - trace_context * (next_addr_segment - addr_segment)
            - not_trace_context * (next_addr_context - addr_context - one),
    );
    yield_constr.constraint(
        segment_range_check
            - trace_segment * (next_addr_virtual - addr_virtual)
            - not_trace_segment * (next_addr_segment - addr_segment - one),
    );
    yield_constr.constraint(
        virtual_range_check
            - trace_virtual * (next_timestamp - timestamp)
            - not_trace_virtual * (next_addr_virtual - addr_virtual - one),
    );

    // Helper constraints to get the product of (1 - trace_context), (1 - trace_segment), and (1 - trace_virtual).
    yield_constr.constraint(two_traces_combined - not_trace_context * not_trace_segment);
    yield_constr.constraint(all_traces_combined - two_traces_combined * not_trace_virtual);

    // Enumerate purportedly-ordered log.
    yield_constr.constraint(next_from - all_traces_combined * to);
}

pub(crate) fn eval_memory_recursively<F: RichField + Extendable<D>, const D: usize>(
    builder: &mut CircuitBuilder<F, D>,
    vars: StarkEvaluationTargets<D, NUM_COLUMNS, NUM_PUBLIC_INPUTS>,
    yield_constr: &mut RecursiveConstraintConsumer<F, D>,
) {
    let one = builder.one_extension();

    let addr_context = vars.local_values[SORTED_MEMORY_ADDR_CONTEXT];
    let addr_segment = vars.local_values[SORTED_MEMORY_ADDR_SEGMENT];
    let addr_virtual = vars.local_values[SORTED_MEMORY_ADDR_VIRTUAL];
    let from = vars.local_values[SORTED_MEMORY_FROM]; // TODO: replace "from" and "to" with "val" and "R/W"
    let to = vars.local_values[SORTED_MEMORY_TO];
    let timestamp = vars.local_values[SORTED_MEMORY_TIMESTAMP];

    let next_addr_context = vars.next_values[SORTED_MEMORY_ADDR_CONTEXT];
    let next_addr_segment = vars.next_values[SORTED_MEMORY_ADDR_SEGMENT];
    let next_addr_virtual = vars.next_values[SORTED_MEMORY_ADDR_VIRTUAL];
    let next_from = vars.next_values[SORTED_MEMORY_FROM];
    let next_to = vars.next_values[SORTED_MEMORY_TO];
    let next_timestamp = vars.next_values[SORTED_MEMORY_TIMESTAMP];

    let trace_context = vars.local_values[MEMORY_TRACE_CONTEXT];
    let trace_segment = vars.local_values[MEMORY_TRACE_SEGMENT];
    let trace_virtual = vars.local_values[MEMORY_TRACE_VIRTUAL];
    let two_traces_combined = vars.local_values[MEMORY_TWO_TRACES_COMBINED];
    let all_traces_combined = vars.local_values[MEMORY_ALL_TRACES_COMBINED];

    let not_trace_context = builder.sub_extension(one, trace_context);
    let not_trace_segment = builder.sub_extension(one, trace_segment);
    let not_trace_virtual = builder.sub_extension(one, trace_virtual);
    let addr_context_diff = builder.sub_extension(next_addr_context, addr_context);
    let addr_segment_diff = builder.sub_extension(next_addr_segment, addr_segment);
    let addr_virtual_diff = builder.sub_extension(next_addr_virtual, addr_virtual);
    let timestamp_diff = builder.sub_extension(next_timestamp, timestamp);

    // First set of ordering constraint: traces are boolean.
    let trace_context_bool = builder.mul_extension(trace_context, not_trace_context);
    yield_constr.constraint(builder, trace_context_bool);
    let trace_segment_bool = builder.mul_extension(trace_segment, not_trace_segment);
    yield_constr.constraint(builder, trace_segment_bool);
    let trace_virtual_bool = builder.mul_extension(trace_virtual, not_trace_virtual);
    yield_constr.constraint(builder, trace_virtual_bool);

    // Second set of ordering constraints: trace matches with no change in corresponding column.
    let cond_context_diff = builder.mul_extension(trace_context, addr_context_diff);
    yield_constr.constraint(builder, cond_context_diff);
    let cond_segment_diff = builder.mul_extension(trace_segment, addr_segment_diff);
    yield_constr.constraint(builder, cond_segment_diff);
    let cond_virtual_diff = builder.mul_extension(trace_virtual, addr_virtual_diff);
    yield_constr.constraint(builder, cond_virtual_diff);

    let context_range_check =
        vars.local_values[crate::registers::range_check_degree::col_rc_degree_input(0)];
    let segment_range_check =
        vars.local_values[crate::registers::range_check_degree::col_rc_degree_input(1)];
    let virtual_range_check =
        vars.local_values[crate::registers::range_check_degree::col_rc_degree_input(2)];

    // Third set of ordering constraints: range-check difference in the column that should be increasing.
    let diff_if_context_equal = builder.mul_extension(trace_context, addr_segment_diff);
    let addr_context_diff_min_one = builder.sub_extension(addr_context_diff, one);
    let diff_if_context_unequal =
        builder.mul_extension(not_trace_context, addr_context_diff_min_one);
    let sum_of_diffs_context =
        builder.add_extension(diff_if_context_equal, diff_if_context_unequal);
    let context_range_check_constraint =
        builder.sub_extension(context_range_check, sum_of_diffs_context);
    yield_constr.constraint(builder, context_range_check_constraint);

    let diff_if_segment_equal = builder.mul_extension(trace_segment, addr_virtual_diff);
    let addr_segment_diff_min_one = builder.sub_extension(addr_segment_diff, one);
    let diff_if_segment_unequal =
        builder.mul_extension(not_trace_segment, addr_segment_diff_min_one);
    let sum_of_diffs_segment =
        builder.add_extension(diff_if_segment_equal, diff_if_segment_unequal);
    let segment_range_check_constraint =
        builder.sub_extension(segment_range_check, sum_of_diffs_segment);
    yield_constr.constraint(builder, segment_range_check_constraint);

    let diff_if_virtual_equal = builder.mul_extension(trace_virtual, timestamp_diff);
    let addr_virtual_diff_min_one = builder.sub_extension(addr_virtual_diff, one);
    let diff_if_virtual_unequal =
        builder.mul_extension(not_trace_virtual, addr_virtual_diff_min_one);
    let sum_of_diffs_virtual =
        builder.add_extension(diff_if_virtual_equal, diff_if_virtual_unequal);
    let virtual_range_check_constraint =
        builder.sub_extension(virtual_range_check, sum_of_diffs_virtual);
    yield_constr.constraint(builder, virtual_range_check_constraint);

    // Helper constraints to get the product of (1 - trace_context), (1 - trace_segment), and (1 - trace_virtual).
    let expected_two_traces_combined = builder.mul_extension(not_trace_context, not_trace_segment);
    let two_traces_diff = builder.sub_extension(two_traces_combined, expected_two_traces_combined);
    yield_constr.constraint(builder, two_traces_diff);
    let expected_all_traces_combined =
        builder.mul_extension(expected_two_traces_combined, not_trace_virtual);
    let all_traces_diff = builder.sub_extension(all_traces_combined, expected_all_traces_combined);
    yield_constr.constraint(builder, all_traces_diff);

    // Enumerate purportedly-ordered log.
    let expected_next_from = builder.mul_extension(all_traces_combined, to);
    let next_from_diff = builder.sub_extension(next_from, expected_next_from);
    yield_constr.constraint(builder, next_from_diff);
}
