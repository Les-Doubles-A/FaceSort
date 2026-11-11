//! Conformité du portage Rust à l'implémentation OpenCV (FaceDetectorYN, FaceRecognizerSF).
//! Référence générée avec OpenCV 5.0.0 : fixtures/reference/opencv_reference.json.

mod common;

use facesort::align::align_face;
use facesort::embed::{cosine, l2_normalize};
use facesort::imageio::decode_oriented;
use serde_json::Value;

#[test]
fn matches_opencv_reference() {
    let Some(mut engine) = common::engine() else { return };
    let reference: Value =
        serde_json::from_str(&std::fs::read_to_string(common::fixtures().join("reference/opencv_reference.json")).unwrap())
            .unwrap();
    for (file, expected) in reference.as_object().unwrap() {
        let img = decode_oriented(&std::fs::read(common::fixtures().join(file)).unwrap()).unwrap();
        let dets = engine.detector.detect(&img).unwrap();
        let exp_faces = expected["faces"].as_array().unwrap();
        assert_eq!(dets.len(), exp_faces.len(), "{file} : nombre de visages");
        for (d, e) in dets.iter().zip(exp_faces) {
            let eb: Vec<f32> = e["box"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            for k in 0..4 {
                assert!((d.bbox[k] - eb[k]).abs() < 1.0, "{file} : cadre {:?} vs {eb:?}", d.bbox);
            }
            assert!((d.score - e["score"].as_f64().unwrap() as f32).abs() < 1e-3, "{file} : score");
            let mut ef: Vec<f32> = e["feature"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            l2_normalize(&mut ef);
            let ours = engine.embedder.embed(&align_face(&img, &d.landmarks)).unwrap();
            let sim = cosine(&ours, &ef);
            assert!(sim > 0.99, "{file} : similarité avec l'empreinte OpenCV = {sim}");
        }
    }
}
