"""Turn an effect file's shader variations into SPIR-V Effect Forge can draw with.

    python eft_shaders.py <shader dir>

<shader dir> holds Visionary's dump_shader_variations output (v<i>_s<stage>.code / .ctl). Steps:
    1. tools/eft_mx/eft_mx.exe translates every stage to Vulkan GLSL (and writes _report.jsonl)
    2. the GLSL is adjusted to wgpu's conventions and to what its SPIR-V reader (naga) accepts:
         - the push-constant block MXPC (alpha test, point size) becomes a storage block at set 0, binding 19 (vertex) or
           39 (fragment), so the pipeline needs no push-constant feature
         - each combined sampler "sampler2D mx_tU_sampler2D" at set 1, binding B is split into a texture at B and a
           sampler at B + 64: naga reads no combined image samplers
         - textureQueryLod (no naga equivalent) becomes the same computation from screen-space derivatives
    3. glslang compiles it to <shader dir>/spv/v<i>_s<stage>.spv
<shader dir>/spv/index.json lists per variation and stage what the draw has to bind (constant buffers, vertex inputs,
samplers); Visionary's eff_shader module reads it.

Bindings (eft_mx): set 0 = vertex constant buffer n at n, fragment at 20 + n; set 1 = fragment texture unit u at u, vertex
at 32 + u, their samplers 64 above.
"""
import concurrent.futures
import json
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
EFT_MX = os.path.join(HERE, "eft_mx", "eft_mx.exe")
GLSLANG = os.environ.get("GLSLANG", os.path.join(HERE, "glslang.exe"))
SAMPLER_SPLIT = 64
MXPC_BINDING = {"vert": 19, "frag": 39}

QLOD = r"""
vec2 mx_qlod_(vec2 uv, vec2 size, float levels) {
  vec2 dx = dFdx(uv * size), dy = dFdy(uv * size);
  float lod = 0.5 * log2(max(dot(dx, dx), dot(dy, dy)));
  return vec2(clamp(lod, 0.0, max(levels - 1.0, 0.0)), lod); }
#define mx_qlod(t, s, uv) mx_qlod_(uv, vec2(textureSize(sampler2D(t, s), 0)), float(textureQueryLevels(sampler2D(t, s))))
"""


def adjust(src, kind):
    # No y flip: replayed against the game's own framebuffer grabs, the game's clip space already puts NDC y = +1 at the
    # top of the picture, as wgpu does (negating y mirrored every effect about the middle of the screen).
    src = src.replace("layout(push_constant) uniform MXPC {",
                      f"layout(std430, set = 0, binding = {MXPC_BINDING[kind]}) readonly buffer MXPC {{")
    # wgpu refuses an infinity constant, so test for it as anything above the largest finite float.
    src = re.sub(r"abs\(F\((r\d+)\)\) == \(F\(0x7f800000u\)\)", r"abs(F(\1)) > (F(0x7f7fffffu))", src)
    names = []

    def split(m):
        binding, typ, name = int(m.group(1)), m.group(2), m.group(3)
        names.append((name, typ))
        tex_type = typ.replace("sampler", "texture", 1)
        return (f"layout(set = 1, binding = {binding}) uniform {tex_type} {name}_t;\n"
                f"layout(set = 1, binding = {binding + SAMPLER_SPLIT}) uniform sampler {name}_s;")

    src = re.sub(r"layout\(set = 1, binding = (\d+)\) uniform (sampler\w+) (\w+);", split, src)
    for name, typ in names:
        src = re.sub(r"textureQueryLod\(%s\s*," % name, f"mx_qlod({name}_t, {name}_s,", src)
        src = re.sub(r"\b%s\b(?!_)" % name, f"{typ}({name}_t, {name}_s)", src)
    if "mx_qlod(" in src:
        # after the #define lines at the top, before any function that uses it
        at = src.index("\n", src.index("#define U(x)")) + 1
        src = src[:at] + QLOD + src[at:]
    return src


def compile_one(job):
    glsl_path, spv_path, kind = job
    src = adjust(open(glsl_path).read(), kind)
    vk_path = spv_path[:-4] + ".vk.glsl"
    open(vk_path, "w").write(src)
    r = subprocess.run([GLSLANG, "-V", "--target-env", "vulkan1.2", "-S", kind, "-o", spv_path, vk_path],
                       capture_output=True, text=True)
    if r.returncode:
        return os.path.basename(glsl_path), (r.stdout + r.stderr).strip().splitlines()[-3:]
    os.remove(vk_path)
    return os.path.basename(glsl_path), None


def main():
    folder = sys.argv[1]
    subprocess.run([EFT_MX, folder, "1"], check=True)
    report = {}
    for line in open(os.path.join(folder, "_report.jsonl")):
        row = json.loads(line)
        report[row["file"][:-5]] = row
    out = os.path.join(folder, "spv")
    os.makedirs(out, exist_ok=True)
    jobs = []
    for base, row in report.items():
        if not row["ok"]:
            continue
        kind = "frag" if base.endswith("_s4") else "vert"
        jobs.append((os.path.join(folder, base + ".glsl"), os.path.join(out, base + ".spv"), kind))
    failed = {}
    with concurrent.futures.ThreadPoolExecutor(os.cpu_count()) as pool:
        for name, err in pool.map(compile_one, jobs):
            if err:
                failed[name] = err
    index = {}
    for base, row in report.items():
        m = re.match(r"v(\d+)_s(\d+)$", base)
        if not m:
            continue
        stage = "fs" if m.group(2) == "4" else "vs"
        ok = row["ok"] and base + ".glsl" not in failed
        entry = {"ok": ok, "cbufs": row.get("cbufs", 0), "in": row.get("in", 0), "out": row.get("out", 0),
                 "alpha": row.get("alpha", 0), "samplers": row.get("samplers", [])}
        if not ok:
            entry["why"] = failed.get(base + ".glsl") or row.get("why")
        index.setdefault(int(m.group(1)), {})[stage] = entry
    json.dump({"sampler_split": SAMPLER_SPLIT, "mxpc_binding": MXPC_BINDING, "variations": index},
              open(os.path.join(out, "index.json"), "w"), indent=0)
    print(f"{len(jobs) - len(failed)} of {len(jobs)} stages compiled to SPIR-V")
    for name, err in list(failed.items())[:10]:
        print(" ", name, "|", " / ".join(err))


if __name__ == "__main__":
    main()
