"""Renders Sofa's welcome scene with Blender (run headless):

    blender -b -P assets/blender/make_assets.py -- <output dir>

A fabric sofa in a dim living room, lit by a floor lamp and the faint glow of a TV.
The left side of the frame stays dark and calm so the setup text can sit on it.
"""
import bpy
import math
import os
import sys
from mathutils import Vector

OUT = sys.argv[sys.argv.index("--") + 1] if "--" in sys.argv else os.path.dirname(__file__)
os.makedirs(OUT, exist_ok=True)


def lin(hex_color):
    h = hex_color.lstrip("#")
    c = [int(h[i:i + 2], 16) / 255 for i in (0, 2, 4)]
    return tuple(((x + 0.055) / 1.055) ** 2.4 if x > 0.04045 else x / 12.92 for x in c) + (1,)


def set_input(node, names, value):
    for n in names:
        if n in node.inputs:
            node.inputs[n].default_value = value
            return


def material(name, color, rough=0.8, sheen=0.0, emission=None, strength=0.0):
    m = bpy.data.materials.new(name)
    m.use_nodes = True
    b = m.node_tree.nodes["Principled BSDF"]
    b.inputs["Base Color"].default_value = lin(color)
    set_input(b, ["Roughness"], rough)
    set_input(b, ["Sheen Weight", "Sheen"], sheen)
    set_input(b, ["Sheen Roughness"], 0.6)
    if emission:
        set_input(b, ["Emission Color", "Emission"], lin(emission))
        set_input(b, ["Emission Strength"], strength)
    return m


def fabric(name, color):
    """Woven look: a fine noise bumps the surface so it reads as cloth, not plastic."""
    m = material(name, color, rough=0.92, sheen=0.6)
    nt = m.node_tree
    b = nt.nodes["Principled BSDF"]
    noise = nt.nodes.new("ShaderNodeTexNoise")
    noise.inputs["Scale"].default_value = 420
    bump = nt.nodes.new("ShaderNodeBump")
    bump.inputs["Strength"].default_value = 0.12
    nt.links.new(noise.outputs["Fac"], bump.inputs["Height"])
    nt.links.new(bump.outputs["Normal"], b.inputs["Normal"])
    return m


def box(name, size, loc, mat, bevel=0.05, subd=0, rot=(0, 0, 0)):
    bpy.ops.mesh.primitive_cube_add(size=1, location=loc, rotation=rot)
    ob = bpy.context.active_object
    ob.name = name
    ob.scale = size
    bpy.ops.object.transform_apply(scale=True)
    if bevel:
        mod = ob.modifiers.new("Bevel", "BEVEL")
        mod.width = bevel
        mod.segments = 6
        mod.limit_method = "NONE"
    if subd:
        s = ob.modifiers.new("Soft", "SUBSURF")
        s.levels = subd
        s.render_levels = subd
    ob.data.materials.append(mat)
    bpy.ops.object.shade_smooth()
    return ob


def cyl(name, r, depth, loc, mat, verts=48):
    bpy.ops.mesh.primitive_cylinder_add(radius=r, depth=depth, location=loc, vertices=verts)
    ob = bpy.context.active_object
    ob.name = name
    ob.data.materials.append(mat)
    bpy.ops.object.shade_smooth()
    return ob


bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
for engine in ("BLENDER_EEVEE", "BLENDER_EEVEE_NEXT"):
    try:
        scene.render.engine = engine
        break
    except TypeError:
        continue
scene.render.resolution_x = 1920
scene.render.resolution_y = 1080
scene.render.image_settings.file_format = "JPEG"
scene.render.image_settings.quality = 90
try:
    scene.view_settings.view_transform = "AgX"
    scene.view_settings.look = "AgX - Medium High Contrast"
except TypeError:
    pass
ee = scene.eevee
for attr, val in (("taa_render_samples", 128), ("use_raytracing", True), ("use_shadows", True),
                  ("use_gtao", True), ("shadow_ray_count", 3), ("shadow_step_count", 12)):
    try:
        setattr(ee, attr, val)
    except AttributeError:
        pass

world = bpy.data.worlds.new("World")
scene.world = world
world.use_nodes = True
world.node_tree.nodes["Background"].inputs["Color"].default_value = lin("#0c0b0a")
world.node_tree.nodes["Background"].inputs["Strength"].default_value = 0.35

# Room
wall = material("Wall", "#2a2420", rough=0.95)
floor = material("Floor", "#3a2a1e", rough=0.55)
rug = fabric("Rug", "#3a2f27")
box("Floor", (30, 30, 0.1), (0, 0, -0.05), floor, bevel=0)
box("WallBack", (30, 0.1, 10), (0, 2.2, 5), wall, bevel=0)
box("WallLeft", (0.1, 30, 10), (-7.5, 0, 5), wall, bevel=0)
box("Rug", (4.6, 2.6, 0.03), (2.0, -0.2, 0.015), rug, bevel=0.01)

# Sofa: sits right of centre, facing the camera / TV
cloth = fabric("Sofa", "#9a5638")
cushion = fabric("Cushion", "#a45e3e")
wood = material("Wood", "#4a2f1c", rough=0.4)
sx = 2.0
box("Base", (3.4, 1.2, 0.42), (sx, 1.0, 0.42), cloth, bevel=0.08)
box("Back", (3.4, 0.36, 0.9), (sx, 1.48, 0.95), cloth, bevel=0.14, subd=1)
box("ArmL", (0.36, 1.2, 0.62), (sx - 1.72, 1.0, 0.62), cloth, bevel=0.14, subd=1)
box("ArmR", (0.36, 1.2, 0.62), (sx + 1.72, 1.0, 0.62), cloth, bevel=0.14, subd=1)
for i, x in enumerate((sx - 0.78, sx + 0.78)):
    box(f"Seat{i}", (1.5, 1.0, 0.2), (x, 0.92, 0.72), cushion, bevel=0.08, subd=1)
    box(f"BackC{i}", (1.45, 0.26, 0.62), (x, 1.22, 1.08), cushion, bevel=0.1, subd=1, rot=(math.radians(-10), 0, 0))
throw = fabric("Throw", "#d8cbb4")
box("Pillow", (0.5, 0.18, 0.46), (sx + 1.25, 1.12, 0.98), throw, bevel=0.08, subd=2, rot=(math.radians(-14), 0, math.radians(-12)))
for x in (sx - 1.6, sx + 1.6):
    for y in (0.5, 1.5):
        cyl("Leg", 0.045, 0.22, (x, y, 0.11), wood, verts=16)

# Side table + floor lamp on the left of the sofa
box("Table", (0.6, 0.6, 0.05), (sx - 2.45, 1.1, 0.62), wood, bevel=0.01)
cyl("TableLeg", 0.03, 0.6, (sx - 2.45, 1.1, 0.3), wood, verts=16)
metal = material("Metal", "#1d1b19", rough=0.35)
cyl("LampPole", 0.02, 1.5, (sx - 2.75, 1.65, 0.75), metal, verts=12)
shade = material("Shade", "#e9dcc3", rough=0.9, emission="#ffb468", strength=3.0)
bpy.ops.mesh.primitive_cone_add(radius1=0.32, radius2=0.2, depth=0.38, location=(sx - 2.75, 1.65, 1.78), vertices=48, end_fill_type="NOTHING")
s = bpy.context.active_object
s.data.materials.append(shade)
s.visible_shadow = False
bpy.ops.object.shade_smooth()

# Lights
def add_light(kind, name, loc, energy, color, size=0.5, rot=None):
    d = bpy.data.lights.new(name, kind)
    d.energy = energy
    d.color = color
    if kind in ("AREA",):
        d.size = size
        d.shape = "DISK"
    elif hasattr(d, "shadow_soft_size"):
        d.shadow_soft_size = size
    ob = bpy.data.objects.new(name, d)
    ob.location = loc
    if rot:
        ob.rotation_euler = rot
    bpy.context.collection.objects.link(ob)
    return ob

add_light("POINT", "Lamp", (sx - 2.75, 1.65, 1.66), 700, (1.0, 0.68, 0.38), size=0.22)
# The TV: a cool, soft rectangle of light from the camera side
tv = add_light("AREA", "TV", (1.6, -4.5, 1.6), 45, (0.62, 0.74, 1.0), size=3.0)
tv.rotation_euler = Vector((0.2, 5.5, -0.2)).to_track_quat("-Z", "Y").to_euler()
add_light("AREA", "Bounce", (4.5, -1.0, 3.5), 40, (1.0, 0.8, 0.6), size=4.0,
          rot=Vector((-4.5, 2.0, -3.5)).to_track_quat("-Z", "Y").to_euler())

# Camera: low, slightly left, so the sofa fills the right and the left stays calm
cam_d = bpy.data.cameras.new("Cam")
cam_d.lens = 32
cam_d.dof.use_dof = True
cam_d.dof.focus_distance = 6.2
cam_d.dof.aperture_fstop = 2.8
cam = bpy.data.objects.new("Cam", cam_d)
bpy.context.collection.objects.link(cam)
cam.location = (-0.6, -5.2, 1.25)
target = Vector((0.8, 1.0, 0.8))
cam.rotation_euler = (target - cam.location).to_track_quat("-Z", "Y").to_euler()
scene.camera = cam

scene.render.filepath = os.path.join(OUT, "welcome.jpg")
bpy.ops.render.render(write_still=True)
print("rendered welcome")
