//! Reproductions of geometry/animation defects from `patch/fixes.patch`.
//! Each test verifies the mathematically expected result in right-handed Java space
//! and Bedrock geometry space.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use rp_core::{
    AnimationFrame, AnimationSpec, ConversionReport, ElementFace, ElementRotation, FaceDirection,
    MappingTable, ModelAsset, ModelCategory, ModelElement, RotationAxis,
};
use rp_transforms::model::{
    BedrockGeoFile, bedrock_geo_to_model_assets, bedrock_to_java_box, bedrock_to_java_face_uv,
    bedrock_to_java_pivot, model_asset_to_bedrock_geo,
};
use rp_transforms::texture::java_animation_to_flipbook;

fn geo(json: &str) -> BedrockGeoFile {
    BedrockGeoFile::from_json_slice(json.as_bytes()).unwrap()
}

fn to_java(json: &str) -> (Vec<ModelElement>, ConversionReport) {
    let mut r = ConversionReport::new();
    let m = bedrock_geo_to_model_assets(&geo(json), "block/t", ModelCategory::Block, &mut r);
    (m[0].elements.clone(), r)
}

fn rot_z_2d(p: [f32; 2], c: [f32; 2], deg: f32) -> [f32; 2] {
    let (s, k) = deg.to_radians().sin_cos();
    let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
    [c[0] + k * dx - s * dy, c[1] + s * dx + k * dy]
}

fn up_face() -> BTreeMap<FaceDirection, ElementFace> {
    BTreeMap::from([(
        FaceDirection::Up,
        ElementFace {
            uv: None,
            texture: "#all".into(),
            cullface: None,
            rotation: 0,
            tintindex: None,
        },
    )])
}

fn java_model(el: ModelElement) -> ModelAsset {
    ModelAsset {
        id: "block/t".into(),
        category: ModelCategory::Block,
        parent: None,
        textures: BTreeMap::from([("all".into(), "block/stone".into())]),
        texture_size: [16, 16],
        ambient_occlusion: true,
        elements: vec![el],
        display: BTreeMap::new(),
        has_overrides: false,
        overrides_count: 0,
    }
}

const FACE: &str = r#""uv": {"north": {"uv": [0,0], "uv_size": [4,4]}}"#;

/// D1: a bone rotation is inherited by child bone cubes with exact world-space placement.
#[test]
fn d1_parent_bone_rotation_is_inherited_by_child_cubes() {
    let (els, report) = to_java(&format!(
        r#"{{
      "format_version":"1.12.0","minecraft:geometry":[{{
        "description":{{"identifier":"geometry.t","texture_width":16,"texture_height":16}},
        "bones":[
          {{"name":"arm","pivot":[0,12,0],"rotation":[0,0,30]}},
          {{"name":"hand","parent":"arm","pivot":[0,6,0],
            "cubes":[{{"origin":[-2,4,-2],"size":[4,4,4],{FACE}}}]}}
        ]}}]}}"#
    ));
    let el = &els[0];
    let rot = el
        .rotation
        .as_ref()
        .expect("child cube must inherit the parent's 30° rotation");
    assert_eq!(rot.axis, RotationAxis::Z);
    assert!((rot.angle - 30.0).abs() < 1e-3, "angle {}", rot.angle);

    // Check world-space corner matches rotating the unrotated Java cube corner 30° about parent pivot (8, 12, 8).
    let (raw_from, _) = bedrock_to_java_box([-2.0, 4.0, -2.0], [4.0, 4.0, 4.0]);
    let parent_pivot = bedrock_to_java_pivot([0.0, 12.0, 0.0]);
    let truth = rot_z_2d([raw_from[0], raw_from[1]], [parent_pivot[0], parent_pivot[1]], 30.0);
    let got = rot_z_2d([el.from[0], el.from[1]], [rot.origin[0], rot.origin[1]], rot.angle);
    assert!(
        (truth[0] - got[0]).abs() < 1e-3 && (truth[1] - got[1]).abs() < 1e-3,
        "truth {truth:?} vs converter {got:?}"
    );
    assert_eq!(report.lossy_count, 0);
}

/// D1b: a bone rotation and a cube rotation on the same bone compose cleanly.
#[test]
fn d1b_bone_and_cube_rotation_compose() {
    let (els, _) = to_java(&format!(
        r#"{{
      "format_version":"1.12.0","minecraft:geometry":[{{
        "description":{{"identifier":"geometry.t","texture_width":16,"texture_height":16}},
        "bones":[{{"name":"b","pivot":[0,8,0],"rotation":[0,0,10],
          "cubes":[{{"origin":[-2,4,-2],"size":[4,4,4],"pivot":[0,8,0],"rotation":[0,0,20],{FACE}}}]}}]}}]}}"#
    ));
    let rot = els[0].rotation.as_ref().unwrap();
    assert_eq!(rot.axis, RotationAxis::Z);
    assert!(
        (rot.angle - 30.0).abs() < 1e-3,
        "10°+20° about the same axis must give 30°, got {}",
        rot.angle
    );
}

/// D2: `inflate` grows the cube by `inflate` on every side.
#[test]
fn d2_inflate_is_applied() {
    let (els, _) = to_java(&format!(
        r#"{{
      "format_version":"1.12.0","minecraft:geometry":[{{
        "description":{{"identifier":"geometry.t","texture_width":16,"texture_height":16}},
        "bones":[{{"name":"b","cubes":[{{"origin":[-2,4,-2],"size":[4,4,4],"inflate":0.5,{FACE}}}]}}]}}]}}"#
    ));
    // plain: from=[6,4,6] to=[10,8,10]; inflated by 0.5 -> [5.5,3.5,5.5]..[10.5,8.5,10.5]
    let e = &els[0];
    for i in 0..3 {
        assert!(
            (e.to[i] - e.from[i] - 5.0).abs() < 1e-4,
            "axis {i}: size {}",
            e.to[i] - e.from[i]
        );
    }
    assert!((e.from[1] - 3.5).abs() < 1e-4);
}

/// D3: Java `rescale:true` scales the cube by 1/cos(angle) on the two non-rotation axes.
#[test]
fn d3_rescale_is_applied_when_exporting() {
    let el = ModelElement {
        group: None,
        from: [4.0, 4.0, 4.0],
        to: [12.0, 12.0, 12.0],
        rotation: Some(ElementRotation {
            origin: [8.0, 8.0, 8.0],
            axis: RotationAxis::Y,
            angle: 45.0,
            rescale: true,
            extra_bedrock_euler: None,
        }),
        faces: up_face(),
        shade: true,
        shade_direction_override: None,
        light_emission: None,
    };
    let mut r = ConversionReport::new();
    let g = model_asset_to_bedrock_geo(
        &java_model(el),
        &MappingTable::default_embedded().unwrap(),
        &mut r,
    );
    let c = &g.geometries[0].bones[0].cubes[0];
    let s = std::f32::consts::SQRT_2; // 1/cos(45°)
    assert!((c.size[0] - 8.0 * s).abs() < 1e-3, "x size {}", c.size[0]);
    assert!((c.size[2] - 8.0 * s).abs() < 1e-3, "z size {}", c.size[2]);
    assert!(
        (c.size[1] - 8.0).abs() < 1e-4,
        "y (rotation axis) must be untouched"
    );
}

/// D4: Java's implicit UVs are face specific (vanilla `BlockElement.uvsByFace`),
/// with Bedrock's 180° up/down face UV flip preserved on export.
#[test]
fn d4_default_uv_per_face_matches_vanilla() {
    let mut faces = BTreeMap::new();
    for d in [
        FaceDirection::North,
        FaceDirection::East,
        FaceDirection::Down,
        FaceDirection::South,
    ] {
        faces.insert(
            d,
            ElementFace {
                uv: None,
                texture: "#all".into(),
                cullface: None,
                rotation: 0,
                tintindex: None,
            },
        );
    }
    let el = ModelElement {
        group: None,
        from: [2.0, 3.0, 5.0],
        to: [6.0, 9.0, 11.0],
        rotation: None,
        faces,
        shade: true,
        shade_direction_override: None,
        light_emission: None,
    };
    let mut r = ConversionReport::new();
    let g = model_asset_to_bedrock_geo(
        &java_model(el),
        &MappingTable::default_embedded().unwrap(),
        &mut r,
    );
    let c = &g.geometries[0].bones[0].cubes[0];
    let rp_transforms::model::BedrockCubeUv::PerFace(m) = &c.uv else {
        panic!("per-face expected")
    };
    // north: [16-x2, 16-y2, 16-x1, 16-y1] = [10, 7, 14, 13] -> uv (10,7), size (4,6)
    assert_eq!(
        (m["north"].uv, m["north"].uv_size),
        ([10.0, 7.0], Some([4.0, 6.0]))
    );
    // east:  [16-z2, 16-y2, 16-z1, 16-y1] = [5, 7, 11, 13] -> uv (5,7), size (6,6)
    assert_eq!(
        (m["east"].uv, m["east"].uv_size),
        ([5.0, 7.0], Some([6.0, 6.0]))
    );
    // south: [x1, 16-y2, x2, 16-y1] = [2, 7, 6, 13] -> uv (2,7), size (4,6)
    assert_eq!(
        (m["south"].uv, m["south"].uv_size),
        ([2.0, 7.0], Some([4.0, 6.0]))
    );
    // down: Java [x1, 16-z2, x2, 16-z1] = [2, 5, 6, 11], flipped 180° for Bedrock -> uv (6,11), size (-4,-6)
    assert_eq!(
        (m["down"].uv, m["down"].uv_size),
        ([6.0, 11.0], Some([-4.0, -6.0]))
    );
    assert_eq!(
        bedrock_to_java_face_uv(FaceDirection::Down, m["down"].uv, m["down"].uv_size.unwrap()),
        [2.0, 5.0, 6.0, 11.0]
    );
}

/// D5: `interpolate` combined with non-uniform frame times cannot be reproduced by frame
/// repetition (the blend window shrinks from `t` ticks to one tick). It must be reported lossy.
#[test]
fn d5_interpolated_nonuniform_animation_is_reported_lossy() {
    let spec = AnimationSpec {
        frametime: 2,
        interpolate: true,
        frames: vec![
            AnimationFrame {
                index: 0,
                time: Some(10),
            },
            AnimationFrame {
                index: 1,
                time: Some(4),
            },
        ],
        frame_width: None,
        frame_height: None,
    };
    let mut r = ConversionReport::new();
    let e = java_animation_to_flipbook("t", "textures/blocks/t", &spec, &mut r);
    assert_eq!(e.ticks_per_frame, 2);
    assert_eq!(e.frames, Some(vec![0, 0, 0, 0, 0, 1, 1]));
    assert_eq!(
        r.lossy_count, 1,
        "interpolation over repeated frames is lossy"
    );
}

/// D6: Bedrock `mirror` flips the box-UV unwrapping (east/west swap + U flip).
#[test]
fn d6_mirror_flag_swaps_east_west_and_flips_u() {
    let src = |mirror: &str| {
        format!(
            r#"{{"format_version":"1.12.0","minecraft:geometry":[{{
        "description":{{"identifier":"geometry.t","texture_width":16,"texture_height":16}},
        "bones":[{{"name":"b",{mirror}"cubes":[{{"origin":[-2,4,-2],"size":[4,4,4],"uv":[0,0]}}]}}]}}]}}"#
        )
    };
    let (plain, _) = to_java(&src(""));
    let (mir, _) = to_java(&src(r#""mirror":true,"#));
    let uv = |e: &ModelElement, d: FaceDirection| e.faces[&d].uv.unwrap();
    // plain east = [0,4,4,8], west = [8,4,12,8], north = [4,4,8,8]
    assert_eq!(uv(&plain[0], FaceDirection::East), [0.0, 4.0, 4.0, 8.0]);
    assert_eq!(uv(&plain[0], FaceDirection::West), [8.0, 4.0, 12.0, 8.0]);
    // mirrored: east takes west's rectangle with u reversed, and vice versa
    assert_eq!(uv(&mir[0], FaceDirection::East), [12.0, 4.0, 8.0, 8.0]);
    assert_eq!(uv(&mir[0], FaceDirection::West), [4.0, 4.0, 0.0, 8.0]);
    assert_eq!(uv(&mir[0], FaceDirection::North), [8.0, 4.0, 4.0, 8.0]);
}

/// Independent numeric check of SE(3) bone chain composition: rotate a corner point-by-point
/// through the bone chain in Java space and compare with the converted element's rotation.
#[test]
fn chain_with_different_pivots_matches_pointwise_rotation() {
    let (els, report) = to_java(&format!(
        r#"{{
      "format_version":"1.12.0","minecraft:geometry":[{{
        "description":{{"identifier":"geometry.t","texture_width":16,"texture_height":16}},
        "bones":[
          {{"name":"a","pivot":[0,12,0],"rotation":[0,0,30]}},
          {{"name":"b","parent":"a","pivot":[4,6,0],"rotation":[0,0,-10],
            "cubes":[{{"origin":[1,2,-2],"size":[4,4,4],{FACE}}}]}}
        ]}}]}}"#
    ));
    assert_eq!(report.lossy_count, 0);
    let el = &els[0];
    let r = el.rotation.as_ref().expect("net 20° rotation");
    assert_eq!(r.axis, RotationAxis::Z);
    assert!((r.angle - 20.0).abs() < 1e-3, "angle {}", r.angle);

    // Ground truth in right-handed Java space:
    // bone b (-10° about Java pivot (4, 6)), then bone a (+30° about Java pivot (8, 12)).
    let (raw_from, _) = bedrock_to_java_box([1.0, 2.0, -2.0], [4.0, 4.0, 4.0]);
    let piv_b = bedrock_to_java_pivot([4.0, 6.0, 0.0]);
    let piv_a = bedrock_to_java_pivot([0.0, 12.0, 0.0]);
    let truth = rot_z_2d(
        rot_z_2d([raw_from[0], raw_from[1]], [piv_b[0], piv_b[1]], -10.0),
        [piv_a[0], piv_a[1]],
        30.0,
    );
    let got = rot_z_2d([el.from[0], el.from[1]], [r.origin[0], r.origin[1]], r.angle);
    assert!(
        (truth[0] - got[0]).abs() < 1e-3 && (truth[1] - got[1]).abs() < 1e-3,
        "truth {truth:?} vs converter {got:?}"
    );
}

/// Rotations that cancel leave only a pure translation, which is baked into the cuboid.
#[test]
fn cancelling_rotations_bake_translation() {
    let (els, _) = to_java(&format!(
        r#"{{
      "format_version":"1.12.0","minecraft:geometry":[{{
        "description":{{"identifier":"geometry.t","texture_width":16,"texture_height":16}},
        "bones":[
          {{"name":"a","pivot":[0,0,0],"rotation":[0,0,90]}},
          {{"name":"b","parent":"a","pivot":[4,0,0],"rotation":[0,0,-90],
            "cubes":[{{"origin":[1,2,-2],"size":[2,2,2],{FACE}}}]}}
        ]}}]}}"#
    ));
    assert!(els[0].rotation.is_none());
    let e = &els[0];
    assert!((e.to[0] - e.from[0] - 2.0).abs() < 1e-4);

    // Verify exact Java-space baked translation of raw_from:
    // bone b (-90° about Java (4, 0)), then bone a (+90° about Java (8, 0)).
    let (raw_from, _) = bedrock_to_java_box([1.0, 2.0, -2.0], [2.0, 2.0, 2.0]);
    let piv_b = bedrock_to_java_pivot([4.0, 0.0, 0.0]);
    let piv_a = bedrock_to_java_pivot([0.0, 0.0, 0.0]);
    let truth = rot_z_2d(
        rot_z_2d([raw_from[0], raw_from[1]], [piv_b[0], piv_b[1]], -90.0),
        [piv_a[0], piv_a[1]],
        90.0,
    );
    assert!(
        (e.from[0] - truth[0]).abs() < 1e-3
            && (e.from[1] - truth[1]).abs() < 1e-3
            && (e.from[2] - raw_from[2]).abs() < 1e-3,
        "from {:?} vs truth ({:?}, {})",
        e.from,
        truth,
        raw_from[2]
    );
}
