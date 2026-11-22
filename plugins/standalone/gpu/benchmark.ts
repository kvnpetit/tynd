import type { BenchmarkCategory, BenchmarkResult } from "./client";
import { releaseWebGLContext } from "./webgl";

const MATMUL_DIM = 256;

// TypeScript's DOM lib exposes these as numeric aliases, not namespaces.
// https://www.w3.org/TR/webgpu/#typedefdef-gpubufferusageflags
const BUFFER_USAGE_MAP_READ = 0x0001;
const BUFFER_USAGE_COPY_SRC = 0x0004;
const BUFFER_USAGE_COPY_DST = 0x0008;
const BUFFER_USAGE_STORAGE = 0x0080;
const MAP_MODE_READ = 0x0001;
const TARGET_DURATION_MS = 250;
const MAX_DURATION_MS = 1500;

const MATMUL_SHADER = /* wgsl */ `
  @group(0) @binding(0) var<storage, read> a : array<f32>;
  @group(0) @binding(1) var<storage, read> b : array<f32>;
  @group(0) @binding(2) var<storage, read_write> out : array<f32>;

  const DIM : u32 = ${MATMUL_DIM}u;

  @compute @workgroup_size(8, 8)
  fn main(@builtin(global_invocation_id) gid : vec3<u32>) {
    if (gid.x >= DIM || gid.y >= DIM) { return; }
    var acc : f32 = 0.0;
    for (var k : u32 = 0u; k < DIM; k = k + 1u) {
      acc = acc + a[gid.y * DIM + k] * b[k * DIM + gid.x];
    }
    out[gid.y * DIM + gid.x] = acc;
  }
`;

const WEBGL_VERTEX_SHADER = `
  attribute vec2 aPos;
  void main() { gl_Position = vec4(aPos, 0.0, 1.0); }
`;

const WEBGL_FRAGMENT_SHADER = `
  precision mediump float;
  uniform float uTime;
  void main() {
    float v = uTime;
    for (int i = 0; i < 16; i++) {
      v = v + sin(v * 1.27 + 0.1) * cos(v * 0.93);
    }
    gl_FragColor = vec4(fract(v), fract(v * 1.7), fract(v * 2.3), 1.0);
  }
`;

export async function runBenchmark(): Promise<BenchmarkResult> {
  if (typeof navigator !== "undefined" && navigator.gpu) {
    try {
      const adapter = await navigator.gpu.requestAdapter({ powerPreference: "high-performance" });
      if (adapter) return await runWebGPUMatmul(adapter);
    } catch {
      // Fall back to WebGL when WebGPU exists but adapter/device setup fails.
    }
  }
  return runWebGLFragmentLoad();
}

interface WebGpuWorkload {
  outBuffer: GPUBuffer;
  stagingBuffer: GPUBuffer;
  pipeline: GPUComputePipeline;
  bindGroup: GPUBindGroup;
  bytes: number;
  groupCount: number;
  opsPerDispatch: number;
}

interface BenchmarkSample {
  iterations: number;
  durationMs: number;
}

async function runWebGPUMatmul(adapter: GPUAdapter): Promise<BenchmarkResult> {
  const device = await adapter.requestDevice();
  try {
    const sample = await measureWebGpu(device, createWebGpuWorkload(device));
    device.destroy();
    return benchmarkResult("webgpu", sample, scoreWebGPU);
  } catch {
    device.destroy();
    return runWebGLFragmentLoad();
  }
}

function createWebGpuWorkload(device: GPUDevice): WebGpuWorkload {
  const elements = MATMUL_DIM * MATMUL_DIM;
  const bytes = elements * 4;
  const aBuffer = device.createBuffer({
    size: bytes,
    usage: BUFFER_USAGE_STORAGE | BUFFER_USAGE_COPY_DST,
  });
  const bBuffer = device.createBuffer({
    size: bytes,
    usage: BUFFER_USAGE_STORAGE | BUFFER_USAGE_COPY_DST,
  });
  const outBuffer = device.createBuffer({
    size: bytes,
    usage: BUFFER_USAGE_STORAGE | BUFFER_USAGE_COPY_SRC,
  });
  const stagingBuffer = device.createBuffer({
    size: bytes,
    usage: BUFFER_USAGE_MAP_READ | BUFFER_USAGE_COPY_DST,
  });
  device.queue.writeBuffer(aBuffer, 0, matrixInput(elements, 13, 1));
  device.queue.writeBuffer(bBuffer, 0, matrixInput(elements, 17, 7));

  const pipeline = device.createComputePipeline({
    layout: "auto",
    compute: { module: device.createShaderModule({ code: MATMUL_SHADER }), entryPoint: "main" },
  });
  const bindGroup = device.createBindGroup({
    layout: pipeline.getBindGroupLayout(0),
    entries: [
      { binding: 0, resource: { buffer: aBuffer } },
      { binding: 1, resource: { buffer: bBuffer } },
      { binding: 2, resource: { buffer: outBuffer } },
    ],
  });
  return {
    outBuffer,
    stagingBuffer,
    pipeline,
    bindGroup,
    bytes,
    groupCount: Math.ceil(MATMUL_DIM / 8),
    opsPerDispatch: MATMUL_DIM ** 3,
  };
}

function matrixInput(elements: number, modulus: number, multiplier: number): Float32Array {
  const values = new Float32Array(elements);
  for (let index = 0; index < elements; index++) {
    values[index] = ((index * multiplier) % modulus) * 0.1;
  }
  return values;
}

async function measureWebGpu(
  device: GPUDevice,
  workload: WebGpuWorkload,
): Promise<BenchmarkSample> {
  let iterations = 0;
  const start = performance.now();
  let lastReadback = 0;
  while (true) {
    const encoder = device.createCommandEncoder();
    const pass = encoder.beginComputePass();
    pass.setPipeline(workload.pipeline);
    pass.setBindGroup(0, workload.bindGroup);
    pass.dispatchWorkgroups(workload.groupCount, workload.groupCount);
    pass.end();
    const elapsed = performance.now() - start;
    const shouldReadback = elapsed - lastReadback > 50 || iterations === 0;
    if (shouldReadback) {
      encoder.copyBufferToBuffer(workload.outBuffer, 0, workload.stagingBuffer, 0, workload.bytes);
    }
    device.queue.submit([encoder.finish()]);
    iterations++;
    if (shouldReadback) {
      await waitForReadback(
        workload.stagingBuffer.mapAsync(MAP_MODE_READ),
        Math.max(1, MAX_DURATION_MS - elapsed),
      );
      workload.stagingBuffer.unmap();
      lastReadback = performance.now() - start;
    }
    const durationMs = performance.now() - start;
    if (durationMs >= TARGET_DURATION_MS || durationMs >= MAX_DURATION_MS) {
      return { iterations: iterations * workload.opsPerDispatch, durationMs };
    }
  }
}
async function waitForReadback(readback: Promise<unknown>, timeoutMs: number): Promise<void> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    await Promise.race([
      readback,
      new Promise<never>((_, reject) => {
        timer = setTimeout(
          () => reject(new Error(`WebGPU readback timed out after ${timeoutMs}ms`)),
          timeoutMs,
        );
      }),
    ]);
  } finally {
    if (timer !== undefined) clearTimeout(timer);
  }
}

interface WebGlWorkload {
  gl: WebGLRenderingContext | WebGL2RenderingContext;
  program: WebGLProgram;
  buffer: WebGLBuffer;
  timeUniform: WebGLUniformLocation | null;
}

function runWebGLFragmentLoad(): BenchmarkResult {
  const workload = createWebGlWorkload();
  if (!workload) return zeroResult("webgl");
  try {
    return benchmarkResult("webgl", measureWebGl(workload), scoreWebGL);
  } finally {
    workload.gl.deleteBuffer(workload.buffer);
    workload.gl.deleteProgram(workload.program);
    releaseWebGLContext(workload.gl);
  }
}

function createWebGlWorkload(): WebGlWorkload | null {
  if (typeof document === "undefined") return null;
  const canvas = document.createElement("canvas");
  canvas.width = 512;
  canvas.height = 512;
  const gl = canvas.getContext("webgl2") ?? canvas.getContext("webgl");
  if (!gl) return null;
  const program = compileProgram(gl, WEBGL_VERTEX_SHADER, WEBGL_FRAGMENT_SHADER);
  if (!program) {
    releaseWebGLContext(gl);
    return null;
  }
  const buffer = gl.createBuffer();
  if (!buffer) {
    gl.deleteProgram(program);
    releaseWebGLContext(gl);
    return null;
  }
  gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, -1, 1, 1, -1, 1]),
    gl.STATIC_DRAW,
  );
  const position = gl.getAttribLocation(program, "aPos");
  gl.enableVertexAttribArray(position);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
  gl.useProgram(program);
  return { gl, program, buffer, timeUniform: gl.getUniformLocation(program, "uTime") };
}

function measureWebGl({ gl, timeUniform }: WebGlWorkload): BenchmarkSample {
  let iterations = 0;
  const start = performance.now();
  while (true) {
    gl.uniform1f(timeUniform, iterations * 0.0173);
    gl.drawArrays(gl.TRIANGLES, 0, 6);
    iterations++;
    if (iterations % 8 !== 0) continue;
    gl.finish();
    const elapsed = performance.now() - start;
    if (elapsed >= TARGET_DURATION_MS || elapsed >= MAX_DURATION_MS) break;
  }
  gl.finish();
  return { iterations: iterations * 512 * 512, durationMs: performance.now() - start };
}

function benchmarkResult(
  backend: "webgpu" | "webgl",
  sample: BenchmarkSample,
  scoreFor: (opsPerSec: number) => number,
): BenchmarkResult {
  const opsPerSec = sample.iterations / (sample.durationMs / 1000);
  const score = scoreFor(opsPerSec);
  return {
    durationMs: sample.durationMs,
    opsPerSec,
    score,
    category: categorize(score),
    backend,
  };
}
function compileProgram(
  gl: WebGLRenderingContext | WebGL2RenderingContext,
  vsSrc: string,
  fsSrc: string,
): WebGLProgram | null {
  const vs = compileShader(gl, gl.VERTEX_SHADER, vsSrc);
  const fs = compileShader(gl, gl.FRAGMENT_SHADER, fsSrc);
  if (!vs || !fs) {
    if (vs) gl.deleteShader(vs);
    if (fs) gl.deleteShader(fs);
    return null;
  }
  const program = gl.createProgram();
  if (!program) {
    gl.deleteShader(vs);
    gl.deleteShader(fs);
    return null;
  }
  gl.attachShader(program, vs);
  gl.attachShader(program, fs);
  gl.linkProgram(program);
  gl.deleteShader(vs);
  gl.deleteShader(fs);
  if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
    gl.deleteProgram(program);
    return null;
  }
  return program;
}

function compileShader(
  gl: WebGLRenderingContext | WebGL2RenderingContext,
  type: number,
  src: string,
): WebGLShader | null {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, src);
  gl.compileShader(shader);
  if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
    gl.deleteShader(shader);
    return null;
  }
  return shader;
}

function scoreWebGPU(opsPerSec: number): number {
  const gflops = opsPerSec / 1e9;
  if (gflops <= 0) return 0;
  return clamp(Math.round(50 + 18 * Math.log10(Math.max(gflops, 0.01))), 0, 100);
}

function scoreWebGL(opsPerSec: number): number {
  const billion = opsPerSec / 1e9;
  if (billion <= 0) return 0;
  return clamp(Math.round(20 + 25 * Math.log10(Math.max(billion, 0.01))), 0, 100);
}

function categorize(score: number): BenchmarkCategory {
  if (score < 25) return "low";
  if (score < 50) return "medium";
  if (score < 80) return "high";
  return "extreme";
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
}

function zeroResult(backend: "webgpu" | "webgl"): BenchmarkResult {
  return {
    durationMs: 0,
    opsPerSec: 0,
    score: 0,
    category: "low",
    backend,
  };
}
