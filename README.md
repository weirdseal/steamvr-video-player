# SteamVR Video Player

Windows 10/11 x64에서 실행되는 Rust 동영상 플레이어입니다. egui와 OpenGL을 사용하며, SteamVR을 활성 OpenXR 런타임으로 설정하면 현재 재생 중인 영상을 헤드셋에 출력합니다.

## 빌드와 실행

Rust 1.88 이상, Windows x64 Rust MSVC 툴체인, Visual Studio C++ Build Tools와 Windows SDK가 필요합니다. PowerShell에서 아래 명령을 실행합니다.

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build.ps1
& .\target\x86_64-pc-windows-msvc\release\steamvr-video-player.exe
```

스크립트는 공식 libmpv 0.42.0 LGPL 빌드와 Khronos OpenXR Loader 1.1.65의 SHA-256을 검증한 뒤 실행 파일 옆에 DLL을 준비합니다. 실행 파일을 다른 폴더로 옮길 때 `libmpv-2.dll`과 `openxr_loader.dll`을 함께 옮기세요. 디버그 빌드에는 `-DebugBuild`, 빌드 폴더를 지정하려면 `-BuildDirectory`를 사용합니다. [libmpv 릴리스](https://github.com/mpv-player/mpv/releases/tag/v0.42.0), [OpenXR Loader 릴리스](https://github.com/KhronosGroup/OpenXR-SDK/releases/tag/release-1.1.65)

MinGW-w64 GCC가 PATH에 있다면 GNU 툴체인으로도 빌드할 수 있습니다.

```powershell
rustup toolchain install stable-x86_64-pc-windows-gnu --profile minimal
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\build.ps1 -Gnu
& .\target\x86_64-pc-windows-gnu\release\steamvr-video-player.exe
```

PC 재생에는 데스크톱 OpenGL 3.3 이상이 필요합니다. VR 모드에는 Windows OpenGL 바인딩(`XR_KHR_opengl_enable`)을 제공하는 OpenXR 런타임과 연결된 헤드셋이 필요합니다. 플레이어는 런타임이 지원하는 OpenGL 버전 범위를 조회해 VR 컨텍스트를 생성합니다. [OpenXR Windows OpenGL 바인딩](https://registry.khronos.org/OpenXR/specs/1.1/man/html/XrGraphicsBindingOpenGLWin32KHR.html)

## 사용법

1. **파일 열기**로 MP4, MKV, MOV, AVI 등 libmpv가 지원하는 동영상을 선택하거나 동영상 파일을 창에 끌어다 놓습니다. 실행 파일에 파일 경로를 인수로 넘길 수도 있습니다.
2. SteamVR을 실행하고 SteamVR 설정에서 SteamVR을 현재 OpenXR 런타임으로 지정합니다. 플레이어에서 **연결 확인**을 누르면 런타임과 헤드셋 이름을 확인할 수 있습니다.
3. **설정** 아이콘의 **VR** 탭에서 투영과 입체 형식을 각각 선택한 뒤 **VR 시작**을 누릅니다. VR 재생 중에도 PC에서 탐색, 볼륨, 일시정지를 제어할 수 있습니다. **VR 종료**를 누르면 같은 위치와 재생 상태에서 PC 재생이 이어집니다.

현재 영상의 제목을 PC와 VR에 표시합니다. 파일에 제목이 들어 있으면 그 제목을 사용하고, 제목이 없으면 파일명을 표시합니다. **이전 영상**과 **다음 영상**은 현재 폴더의 정렬 순서를 따릅니다. 이름 오름차순에서는 `2화.mp4` 다음이 `10화.mp4`입니다.

PC와 VR의 **설정** 아이콘에서 한국어와 English, 정렬과 탐색기 보기, 시작 음량과 조절 단위, 자동 재생, VR 화면과 패널 표시 시간을 바꿀 수 있습니다. 선택은 `%LOCALAPPDATA%\SteamVR Video Player\settings.json`에 저장됩니다.

**설정 → 재생**에서 **PC 스킵 간격**과 **VR 스킵 간격**을 각각 설정합니다. 5·10·15·30초를 바로 선택하거나 `−`와 `+`로 1~60초 사이에서 조절할 수 있습니다. PC 값은 스킵 버튼과 좌우 방향키에, VR 값은 재생 패널의 스킵 버튼과 컨트롤러의 좌우 스틱에 적용됩니다.

| 설정 | 동작 |
| --- | --- |
| 평면 | 시작할 때 바라본 방향에 평면 스크린을 배치합니다. **화면 폭**, **거리**를 미터 단위로 변경할 수 있습니다. |
| 180° / 360° | 정거원통형(equirectangular) 영상의 각도 범위입니다. 헤드셋 회전에 따라 시점이 바뀝니다. |
| 모노 / SBS / TB | SBS는 왼쪽과 오른쪽, TB는 위쪽과 아래쪽을 각각 왼눈과 오른눈에 표시합니다. 세 투영 방식과 개별 조합할 수 있습니다. |
| Half SBS/TB 비율 보정 | 평면 모드에서 가로 또는 세로 방향으로 압축된 스테레오 영상을 풀어 각 눈의 화면 비율을 보정합니다. |

| 단축키 | 기능 |
| --- | --- |
| `Ctrl+O` | 파일 열기 |
| `Space` | 재생 / 일시정지 |
| `←` / `→` | 설정된 PC 간격으로 탐색. 기본 5초 |
| `↑` / `↓` | 설정된 단위로 음량 조절. 기본 5% |
| `M` | 음소거 |
| `F` / `F11` | 전체 화면 전환 |
| `Esc` | 전체 화면 닫기 |
| `V` | VR 시작 / 종료 |

### 헤드셋 안에서 재생 제어

VR 모드가 시작되면 시야 아래에 재생 패널이 잠시 표시됩니다. 패널이 숨겨져 있을 때는 어느 쪽 컨트롤러든 트리거를 눌러 엽니다. 원하는 버튼을 가리킨 다음 트리거를 눌러 선택합니다.

- 패널이 열려 있는 동안에는 패널 바깥에서도 컨트롤러의 조준점이 표시됩니다.
- **폴더 아이콘**을 누르면 재생 패널 바로 위에 탐색기가 열립니다. 탐색하면서 아래에서 재생과 음량을 조절할 수 있습니다. 현재 영상의 폴더에서 시작하며, 영상이 없으면 드라이브 목록을 표시합니다.
- 탐색기 상단에서 **이름**, **수정일**, **크기**와 정렬 방향을 바꿀 수 있습니다. 오른쪽의 목록·바둑판 아이콘은 보기 방식을 바꿉니다. **×**, 열린 폴더 아이콘, 두 패널 바깥 누르기로 탐색기를 닫습니다. 영상을 선택하면 탐색기가 닫힙니다.
- 바둑판뷰는 화면에 들어온 동영상의 실제 프레임으로 썸네일을 만듭니다. 썸네일을 준비하는 동안이나 프레임을 읽을 수 없는 경우에는 영상 아이콘이 표시됩니다.
- 목록이나 바둑판의 항목 영역을 가리키고 트리거를 누른 채 위아래로 끌면 스크롤됩니다. 같은 영역을 가리킨 채 스틱을 위아래로 움직여도 스크롤됩니다. 항목에서 트리거를 짧게 눌렀다 놓으면 해당 파일이나 폴더가 열립니다.
- 진행 막대에서 원하는 위치를 가리켜 트리거를 누릅니다. 누른 채 움직이면 이동할 시간이 표시되고, 트리거를 놓으면 그 위치로 이동합니다.
- 재생·일시정지 아이콘과 설정된 VR 이동 간격 버튼으로 재생을 조작합니다. 기본 이동 간격은 10초입니다.
- 음량 증감 버튼과 스피커 아이콘으로 음량과 음소거를 조절합니다.
- 탐색기를 닫은 뒤에는 재생 패널 바깥을 가리켜 트리거를 누르거나 잠시 가리키지 않으면 재생 패널이 숨겨집니다.

패널이 숨겨져 있을 때도 스틱으로 영상과 시점을 조절할 수 있습니다.

| 컨트롤러 | 동작 |
| --- | --- |
| 스틱 위·아래 | 확대·축소 |
| 스틱 좌·우 | 설정된 VR 간격으로 뒤로·앞으로 이동. 스틱을 중앙으로 놓으면 다시 이동할 수 있습니다. |
| 스틱 클릭 | 현재 바라보는 방향을 정면으로 맞추고 확대율을 초기화 |
| 한 손의 잡기 버튼을 누른 채 어느 쪽 스틱이든 이동 | 상하좌우로 시점 회전 |

피코 컨트롤러의 잡기 버튼은 손잡이 옆에서 중지로 누르는 버튼입니다.

PC와 헤드셋의 재생 위치, 재생 상태, 음량은 같은 libmpv 세션에 반영됩니다. 헤드셋에서 조작하려면 SteamVR 대시보드를 닫고 플레이어 화면으로 돌아옵니다. [OpenXR 컨트롤러 입력](https://registry.khronos.org/OpenXR/specs/1.1/man/html/xrSyncActions.html)

## 렌더링

libmpv의 한 재생 세션이 OpenGL 텍스처에 프레임을 렌더링합니다. PC 미리보기는 이 텍스처를 직접 그립니다. VR 모드에서는 별도 스레드가 WGL 공유 컨텍스트에서 같은 텍스처를 읽고, GPU 펜스로 프레임 쓰기와 읽기 순서를 맞춥니다. OpenXR이 예측한 표시 시점의 눈별 위치, 회전과 시야각을 적용해 양안 스왑체인에 직접 렌더링합니다. 디코더의 하드웨어 가속은 libmpv의 `hwdec=auto`가 코덱과 GPU에 맞춰 선택합니다. [libmpv OpenGL 렌더 API](https://github.com/mpv-player/mpv/blob/v0.42.0/include/mpv/render_gl.h), [OpenXR 프레임 제출](https://github.com/KhronosGroup/OpenXR-Guide/blob/main/chapters/frame_submission.md)

헤드셋 연결이나 OpenXR 세션 생성에 실패하면 창에 오류가 표시되고 PC 재생은 계속됩니다. 하이브리드 그래픽 환경에서 그래픽 바인딩 오류가 발생하면 Windows 그래픽 설정에서 SteamVR과 플레이어에 같은 GPU를 지정합니다. 오디오는 libmpv가 Windows 출력 장치로 재생합니다.

한국어 UI에는 Noto Sans KR Regular 한글 서브셋을 사용합니다. [Noto Sans KR](https://github.com/google/fonts/tree/main/ofl/notosanskr), [한글 글꼴 라이선스](assets/OFL-NotoSansKR.txt)
