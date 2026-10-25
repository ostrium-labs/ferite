//! Regression coverage for graph extraction, scheduling, planning and host fallback.

use ferrite_strata::{
    Backend, CompileOutcome, DType, ElementType, Error, Graph, GraphBuilder, Op, Shape, ShapeClass,
    StrataSupport, SupportLevel, ValueId,
};
use ferrite_strata_cpu::CpuBackend;
use ferrite_strata_runtime::{Session, Unit, inputs};

fn dtype() -> DType {
    DType::plain(ElementType::F32)
}
fn shape(dims: &[usize]) -> Shape {
    Shape::new(dims).expect("shape")
}
fn node(
    builder: &mut GraphBuilder,
    op: Op,
    operands: &[ValueId],
    dims: &[usize],
    name: &str,
) -> ValueId {
    builder
        .node(
            op,
            operands,
            &[(dtype(), shape(dims))],
            Default::default(),
            name,
        )
        .expect("node")[0]
}

#[derive(Debug)]
struct ExpOnly(CpuBackend);

impl Backend for ExpOnly {
    fn name(&self) -> &str {
        "exp-only"
    }
    fn plugin_version(&self) -> ferrite_strata::PluginVersion {
        self.0.plugin_version()
    }
    fn capabilities(&self) -> &StrataSupport {
        self.0.capabilities()
    }
    fn supports(&self, pattern: &str, dtype: DType, class: ShapeClass) -> SupportLevel {
        if pattern == "ref::exp" {
            self.0.supports(pattern, dtype, class)
        } else {
            SupportLevel::Refuse
        }
    }
    fn compile(&self, graph: &Graph, pattern: &str) -> Result<CompileOutcome, Error> {
        self.0.compile(graph, pattern)
    }
}

#[test]
fn host_producers_and_consumers_surround_backend_work() {
    let mut builder = GraphBuilder::new("interleaved");
    let input = builder.input(dtype(), shape(&[2]), "input").expect("input");
    let logged = node(&mut builder, Op::Log, &[input], &[2], "log");
    let exponential = node(&mut builder, Op::Exp, &[logged], &[2], "exp");
    let output = node(&mut builder, Op::Log, &[exponential], &[2], "log-again");
    builder.output(output).expect("output");
    let mut session = Session::new();
    session
        .add(Box::new(ExpOnly(CpuBackend::new())))
        .expect("backend");
    let step = session.compile(&builder.build()).expect("compile");
    assert_eq!(step.units(), vec![Unit::Host, Unit::Backend(0), Unit::Host]);
    let arena = step
        .run_with_inputs(&inputs(&[(input, vec![1.0, std::f32::consts::E])]))
        .expect("run");
    let result = arena.f32_at(output).expect("output");
    assert!(result[0].abs() < 1e-6 && (result[1] - 1.0).abs() < 1e-6);
}

#[test]
fn backend_partitions_run_in_dependency_order_not_selection_order() {
    let mut builder = GraphBuilder::new("backend-order");
    let input = builder.input(dtype(), shape(&[2]), "input").expect("input");
    let logged = node(&mut builder, Op::Log, &[input], &[2], "log");
    let first = node(&mut builder, Op::Exp, &[logged], &[2], "exp1");
    let output = node(&mut builder, Op::Exp, &[first], &[2], "exp2");
    builder.output(output).expect("output");
    let mut session = Session::new();
    session
        .add(Box::new(ExpOnly(CpuBackend::new())))
        .expect("backend");
    session.add(Box::new(CpuBackend::new())).expect("backend");
    let step = session.compile(&builder.build()).expect("compile");
    assert_eq!(step.units().first(), Some(&Unit::Backend(1)));
    let arena = step
        .run_with_inputs(&inputs(&[(input, vec![1.0; 2])]))
        .expect("run");
    assert_eq!(
        arena.f32_at(output).expect("output"),
        &[std::f32::consts::E; 2]
    );
}

#[test]
fn same_named_graphs_and_backend_registration_never_reuse_stale_plans() {
    let mut session = Session::new();
    for count in [1, 2] {
        let mut builder = GraphBuilder::unnamed();
        let input = builder.input(dtype(), shape(&[1]), "input").expect("input");
        let mut output = input;
        for index in 0..count {
            output = node(
                &mut builder,
                Op::Exp,
                &[output],
                &[1],
                &format!("exp{index}"),
            );
        }
        builder.output(output).expect("output");
        let graph = builder.build();
        let step = session.compile(&graph).expect("compile");
        assert!(
            step.run_with_inputs(&inputs(&[(input, vec![0.0])]))
                .expect("run")
                .f32_at(output)
                .is_some()
        );
        session.add(Box::new(CpuBackend::new())).expect("backend");
        assert_eq!(
            session
                .compile(&graph)
                .expect("compile with backend")
                .host_unit_count(),
            0
        );
    }
}

#[test]
fn extraction_preserves_linear_weights_and_layer_norm_parameters() {
    let mut builder = GraphBuilder::new("parameters");
    let unused = builder
        .input(dtype(), shape(&[1]), "unused")
        .expect("unused");
    let input = builder
        .input(dtype(), shape(&[2, 2]), "input")
        .expect("input");
    let weight = builder
        .input(dtype(), shape(&[2, 2]), "weight")
        .expect("weight");
    let bias = builder.input(dtype(), shape(&[2]), "bias").expect("bias");
    let gamma = builder.input(dtype(), shape(&[2]), "gamma").expect("gamma");
    let beta = builder.input(dtype(), shape(&[2]), "beta").expect("beta");
    let linear = node(
        &mut builder,
        Op::Linear {
            weight,
            bias: Some(bias),
        },
        &[input],
        &[2, 2],
        "linear",
    );
    let output = node(
        &mut builder,
        Op::LayerNorm {
            gamma,
            beta,
            eps: 0.0,
        },
        &[linear],
        &[2, 2],
        "norm",
    );
    builder.output(linear).expect("linear output");
    builder.output(output).expect("norm output");
    let mut session = Session::new();
    session.add(Box::new(CpuBackend::new())).expect("backend");
    let step = session.compile(&builder.build()).expect("compile");
    let arena = step
        .run_with_inputs(&inputs(&[
            (input, vec![1.0, 3.0, 2.0, 4.0]),
            (weight, vec![1.0, 0.0, 0.0, 1.0]),
            (bias, vec![0.0; 2]),
            (gamma, vec![2.0; 2]),
            (beta, vec![1.0; 2]),
        ]))
        .expect("run");
    assert_eq!(arena.f32_at(linear).expect("linear"), &[1.0, 3.0, 2.0, 4.0]);
    assert_eq!(arena.f32_at(output).expect("norm"), &[-1.0, 3.0, -1.0, 3.0]);
    assert!(arena.get(unused).is_none());
}

#[test]
fn host_binary_ops_broadcast_and_reject_bad_input_lengths() {
    for op in [Op::Add, Op::Mul] {
        let mut builder = GraphBuilder::new("broadcast");
        let left = builder
            .input(dtype(), shape(&[2, 2]), "left")
            .expect("left");
        let right = builder
            .input(dtype(), shape(&[1, 2]), "right")
            .expect("right");
        let output = node(&mut builder, op.clone(), &[left, right], &[2, 2], "binary");
        builder.output(output).expect("output");
        let step = Session::new().compile(&builder.build()).expect("compile");
        let arena = step
            .run_with_inputs(&inputs(&[
                (left, vec![1.0, 2.0, 3.0, 4.0]),
                (right, vec![10.0, 20.0]),
            ]))
            .expect("run");
        let expected = if op == Op::Add {
            vec![11.0, 22.0, 13.0, 24.0]
        } else {
            vec![10.0, 40.0, 30.0, 80.0]
        };
        assert_eq!(arena.f32_at(output).expect("output"), expected);
        for data in [vec![1.0], vec![1.0; 5]] {
            assert!(
                step.run_with_inputs(&inputs(&[(left, data), (right, vec![10.0, 20.0])]))
                    .is_err()
            );
        }
    }
}

#[test]
fn parameters_produced_inside_a_fused_partition_are_remapped_not_staged() {
    let mut builder = GraphBuilder::new("computed-weight");
    let input = builder
        .input(dtype(), shape(&[2, 2]), "input")
        .expect("input");
    let base = builder
        .input(dtype(), shape(&[2, 2]), "base")
        .expect("base");
    let weight = node(&mut builder, Op::Exp, &[base], &[2, 2], "weight");
    let output = node(
        &mut builder,
        Op::Linear { weight, bias: None },
        &[input],
        &[2, 2],
        "linear",
    );
    builder.output(output).expect("output");
    let mut session = Session::new();
    session.add(Box::new(CpuBackend::new())).expect("backend");
    let step = session.compile(&builder.build()).expect("compile");
    assert_eq!(step.unit_count(), 1);
    let arena = step
        .run_with_inputs(&inputs(&[
            (input, vec![1.0, 2.0, 3.0, 4.0]),
            (base, vec![0.0; 4]),
        ]))
        .expect("run");
    assert_eq!(arena.f32_at(output).expect("output"), &[3.0, 3.0, 7.0, 7.0]);
}
