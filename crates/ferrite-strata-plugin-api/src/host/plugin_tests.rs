use super::*;
use ferrite_strata::{
    CapabilitySet, DType, DTypeMask, ElementType, GraphBuilder, PatternSupport, Shape, ShapeClass,
    SupportLevel,
};
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Default)]
struct State {
    allocations: AtomicUsize,
    releases: AtomicUsize,
    runs: AtomicUsize,
    frees: AtomicUsize,
    queries: AtomicUsize,
}

unsafe extern "C" fn release_context(ctx: *mut c_void) {
    unsafe { &*ctx.cast::<State>() }
        .frees
        .fetch_add(1, Ordering::SeqCst);
}

unsafe extern "C" fn refuse_capabilities(_: *mut c_void, _: *mut StrataWireBlob) -> i32 {
    status::INVALID_ARGUMENT
}

unsafe extern "C" fn refuse_live(ctx: *mut c_void, _: *const StrataPatternQuery) -> u32 {
    unsafe { &*ctx.cast::<State>() }
        .queries
        .fetch_add(1, Ordering::SeqCst);
    crate::raw::StrataSupportLevel::Refuse as u32
}

unsafe extern "C" fn allocate(
    ctx: *mut c_void,
    _: *const StrataAllocDesc,
    buffer: *mut *mut StrataBuffer,
    _: *mut *mut StrataError,
) -> i32 {
    unsafe { &*ctx.cast::<State>() }
        .allocations
        .fetch_add(1, Ordering::SeqCst);
    unsafe { *buffer = Box::into_raw(Box::new(vec![0u8; 8])).cast() };
    status::OK
}

unsafe extern "C" fn release_buffer(ctx: *mut c_void, buffer: *mut StrataBuffer) -> i32 {
    unsafe { &*ctx.cast::<State>() }
        .releases
        .fetch_add(1, Ordering::SeqCst);
    drop(unsafe { Box::from_raw(buffer.cast::<Vec<u8>>()) });
    status::OK
}

unsafe extern "C" fn write_buffer(
    _: *mut c_void,
    buffer: *mut StrataBuffer,
    offset: u64,
    data: *const u8,
    length: u64,
) -> i32 {
    let destination = unsafe { &mut *buffer.cast::<Vec<u8>>() };
    let source = unsafe { core::slice::from_raw_parts(data, length as usize) };
    destination[offset as usize..offset as usize + source.len()].copy_from_slice(source);
    status::OK
}

unsafe extern "C" fn read_buffer(
    _: *mut c_void,
    buffer: *const StrataBuffer,
    offset: u64,
    data: *mut u8,
    length: u64,
) -> i32 {
    let source = unsafe { &*buffer.cast::<Vec<u8>>() };
    let destination = unsafe { core::slice::from_raw_parts_mut(data, length as usize) };
    destination.copy_from_slice(&source[offset as usize..offset as usize + length as usize]);
    status::OK
}

unsafe extern "C" fn run_copy(
    ctx: *mut c_void,
    args: *const StrataRunArgs,
    _: *mut *mut StrataError,
) -> i32 {
    unsafe { &*ctx.cast::<State>() }
        .runs
        .fetch_add(1, Ordering::SeqCst);
    let args = unsafe { &*args };
    let buffers = unsafe { core::slice::from_raw_parts(args.args, args.arg_count as usize) };
    let source = unsafe { &*buffers[0].buffer.cast::<Vec<u8>>() }.clone();
    unsafe { &mut *buffers[1].buffer.cast::<Vec<u8>>() }.copy_from_slice(&source);
    status::OK
}

fn vtable() -> StrataPluginV1 {
    let mut table: StrataPluginV1 = unsafe { core::mem::zeroed() };
    table.capabilities = Some(refuse_capabilities);
    table.supports = Some(refuse_live);
    table.alloc = Some(allocate);
    table.buffer_release = Some(release_buffer);
    table.buffer_write = Some(write_buffer);
    table.buffer_read = Some(read_buffer);
    table.run = Some(run_copy);
    table
}

fn plugin(state: &State) -> Plugin {
    let dtype = DType::plain(ElementType::F32);
    let capabilities = CapabilitySet::empty("mock")
        .with_dtype_mask(DTypeMask::all())
        .with_pattern(PatternSupport::new(
            "ref::exp",
            dtype,
            ShapeClass::Vector,
            SupportLevel::Fused,
        ));
    Plugin {
        inner: Arc::new(PluginInner {
            library: Library::open("libc.so.6").expect("system libc"),
            vtable: vtable(),
            ctx: core::ptr::from_ref(state).cast_mut().cast(),
            support: StrataSupport::new(capabilities),
            triple: None,
        }),
    }
}

fn executable(plugin: &Plugin, dtype: DType) -> (RemoteExecutable, ValueId, ValueId) {
    let mut builder = GraphBuilder::new("copy");
    let shape = Shape::new(&[2]).expect("shape");
    let input = builder.input(dtype, shape, "input").expect("input");
    let output = builder
        .node(
            ferrite_strata::Op::Exp,
            &[input],
            &[(dtype, shape)],
            Default::default(),
            "op",
        )
        .expect("op")[0];
    builder.output(output).expect("output");
    (
        RemoteExecutable {
            plugin: Arc::clone(&plugin.inner),
            handle: core::ptr::null_mut(),
            label: "copy".into(),
            graph: builder.build(),
            outputs: vec![output],
        },
        input,
        output,
    )
}

#[test]
fn failed_capability_load_releases_initialized_context_once() {
    let state = State::default();
    let context = core::ptr::from_ref(&state).cast_mut().cast();
    let result = (|| {
        let _guard = ContextGuard {
            ctx: context,
            free: Some(release_context),
        };
        read_capabilities(&vtable(), context, "mock")?;
        Ok::<_, PluginError>(())
    })();
    assert!(result.is_err());
    assert_eq!(state.frees.load(Ordering::SeqCst), 1);
}

#[test]
fn backend_trait_and_partitioner_use_live_plugin_support() {
    let state = State::default();
    let plugin = plugin(&state);
    let dtype = DType::plain(ElementType::F32);
    assert_eq!(
        plugin
            .capabilities()
            .supports("ref::exp", dtype, ShapeClass::Vector),
        SupportLevel::Fused
    );
    assert_eq!(
        Backend::supports(&plugin, "ref::exp", dtype, ShapeClass::Vector),
        SupportLevel::Refuse
    );
    let (executable, _, _) = executable(&plugin, dtype);
    let graph = executable.graph.clone();
    drop(executable);
    let backends: Vec<Box<dyn Backend>> = vec![Box::new(plugin)];
    assert!(
        ferrite_strata::Partitioner::new(&backends)
            .partition(&graph)
            .expect("plan")
            .partitions
            .is_empty()
    );
    assert!(state.queries.load(Ordering::SeqCst) >= 2);
}

#[test]
fn opaque_outputs_survive_consecutive_plugin_calls_without_dtype_reinterpretation() {
    for dtype in [
        DType::plain(ElementType::BF16),
        DType::plain(ElementType::I32),
    ] {
        let state = State::default();
        let plugin = plugin(&state);
        let (executable, input, output) = executable(&plugin, dtype);
        let bytes: Vec<u8> = (0..dtype.byte_len(2))
            .map(|index| index as u8 + 1)
            .collect();
        let mut arena = Arena::new();
        arena.insert(input, ArenaEntry::Opaque(bytes.clone()));
        for _ in 0..2 {
            executable.run(&mut arena).expect("run");
            assert_eq!(arena.get(output), Some(&ArenaEntry::Opaque(bytes.clone())));
            arena.insert(input, arena.get(output).expect("output").clone());
        }
        assert_eq!(state.runs.load(Ordering::SeqCst), 2);
        assert_eq!(
            state.allocations.load(Ordering::SeqCst),
            state.releases.load(Ordering::SeqCst)
        );
    }
}

#[test]
fn missing_wrong_length_and_wrong_dtype_inputs_fail_before_any_plugin_allocation() {
    let state = State::default();
    let plugin = plugin(&state);
    let (executable, input, _) = executable(&plugin, DType::plain(ElementType::F32));
    let mut arena = Arena::new();
    assert!(executable.run(&mut arena).is_err());
    for values in [vec![1.0], vec![1.0; 3]] {
        arena.insert_f32(input, values);
        assert!(executable.run(&mut arena).is_err());
    }
    arena.insert(input, ArenaEntry::Opaque(vec![0; 7]));
    assert!(executable.run(&mut arena).is_err());
    assert_eq!(state.allocations.load(Ordering::SeqCst), 0);
    assert_eq!(state.runs.load(Ordering::SeqCst), 0);
    assert!(
        encode_host(
            &ArenaEntry::F32(vec![1.0]),
            DType::plain(ElementType::I32),
            4
        )
        .is_err()
    );
    assert!(matches!(
        decode_host(1i32.to_le_bytes().to_vec(), DType::plain(ElementType::I32)),
        ArenaEntry::Opaque(_)
    ));
}
