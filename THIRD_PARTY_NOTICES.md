# Composants tiers distribués avec FaceSort

| Composant | Fichier(s) | Licence | Source |
|---|---|---|---|
| YuNet (OpenCV Zoo) | `models/face_detection_yunet_2026may.onnx` | MIT, texte dans `models/LICENSE-yunet` | https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet |
| SFace (OpenCV Zoo) | `models/face_recognition_sface_2021dec.onnx` | Apache 2.0, texte dans `models/LICENSE-sface` | https://github.com/opencv/opencv_zoo/tree/main/models/face_recognition_sface |
| AuraFace-v1 (fal.ai), fichier `glintr100.onnx` | `models/auraface_glintr100.onnx` | Apache 2.0, texte dans `models/LICENSE-auraface.md` | https://huggingface.co/fal/AuraFace-v1 |
| ONNX Runtime (Microsoft), lié dans les exécutables | `FaceSort.exe`, `facesort-cli.exe` | MIT | https://github.com/microsoft/onnxruntime |
| DirectML (Microsoft) | `DirectML.dll` | Licence du paquet Microsoft.AI.DirectML : redistribution autorisée au sein d'applications d'apprentissage automatique pour Windows, pas seule | https://www.nuget.org/packages/Microsoft.AI.DirectML/1.15.4/License |
| Runtime Microsoft Visual C++ | `msvcp140*.dll`, `vcruntime140*.dll` | Fichiers redistribuables de Visual Studio (déploiement local à l'application) | https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files |

Les crates Rust utilisées sont listées dans `Cargo.lock`.
