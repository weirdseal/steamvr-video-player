# SteamVR Video Player

[한국어](#한국어)

A Rust video player for Windows 10 and 11 x64 that sends video directly to SteamVR through OpenXR. Desktop playback requires OpenGL 3.3 or newer.

[Windows build and packaging](build/README.md)

## Desktop playback

Select **Open file** or drag a video into the window. The player opens formats supported by libmpv, including MP4, MKV, MOV, and AVI.

| Shortcut | Action |
| --- | --- |
| `Space` | Play or pause |
| `←` / `→` | Skip backward or forward |
| `↑` / `↓` / `M` | Adjust volume or mute |
| `F` / `F11` / `Esc` | Toggle or exit fullscreen |

Set separate PC and VR skip intervals from 1 to 60 seconds in **Settings → Playback**. The defaults are 5 seconds on PC and 10 seconds in VR.

## SteamVR

1. Connect your headset, start SteamVR, and select SteamVR as the active OpenXR runtime. Use **Check headset** to verify the connection.
2. Under **Settings → VR**, choose Flat, 180°, or 360° projection and Mono, SBS, or Top/Bottom stereo. Select **Enter VR**. Playback continues from the same position when switching modes.
3. Close the SteamVR dashboard to use the player controls. Press the trigger to show the playback panel. Its folder button opens a sortable browser with list and thumbnail grid views.

| Controller | Action |
| --- | --- |
| Trigger tap | Select a control or open a file. Point outside the panels and tap to dismiss. |
| Trigger drag | Seek on the timeline or scroll through files |
| Stick up or down | Zoom, or scroll while pointing at the browser |
| Stick left or right | Skip by the VR interval |
| Stick click | Recenter the view and reset zoom |
| Grip + stick | Rotate the view |

[Development conventions](CLAUDE.md) · [Noto Sans KR license](assets/OFL-NotoSansKR.txt)

The player source code is licensed under the WTFPL v2. [License text](LICENSE)

---

## 한국어

Windows 10/11 x64용 Rust 동영상 플레이어입니다. OpenXR을 통해 SteamVR 헤드셋으로 영상을 직접 출력합니다. PC 재생에는 OpenGL 3.3 이상이 필요합니다.

[Windows 빌드 및 배포](build/README.md)

### PC 재생

**파일 열기**를 누르거나 영상 파일을 창에 끌어다 놓습니다. MP4, MKV, MOV, AVI 등 libmpv가 지원하는 형식을 재생합니다.

| 단축키 | 동작 |
| --- | --- |
| `Space` | 재생 또는 일시정지 |
| `←` / `→` | 뒤로 또는 앞으로 스킵 |
| `↑` / `↓` / `M` | 음량 조절 또는 음소거 |
| `F` / `F11` / `Esc` | 전체 화면 전환 또는 해제 |

**설정 → 재생**에서 PC와 VR의 스킵 간격을 각각 1~60초로 설정할 수 있습니다. 기본값은 PC 5초, VR 10초입니다.

### SteamVR

1. 헤드셋을 연결하고 SteamVR을 실행한 뒤, SteamVR을 현재 OpenXR 런타임으로 지정합니다. 플레이어의 **연결 확인**으로 연결 상태를 확인합니다.
2. **설정 → VR**에서 평면, 180°, 360° 중 투영 방식을 고르고 모노, SBS, Top/Bottom 중 입체 형식을 선택합니다. **VR 시작**을 누릅니다. 모드를 전환해도 재생 위치가 이어집니다.
3. SteamVR 대시보드를 닫고 트리거를 눌러 재생 패널을 엽니다. 폴더 버튼을 누르면 정렬이 가능한 탐색기가 열리며, 리스트뷰와 썸네일 바둑판뷰를 고를 수 있습니다.

| 컨트롤러 | 동작 |
| --- | --- |
| 트리거 짧게 누르기 | 조작 버튼 선택 또는 파일 열기. 패널 바깥을 가리켜 누르면 패널 닫기. |
| 트리거를 누른 채 끌기 | 진행 막대 탐색 또는 파일 목록 스크롤 |
| 스틱 위·아래 | 확대·축소. 탐색기를 가리키는 동안 스크롤. |
| 스틱 좌·우 | VR 간격으로 스킵 |
| 스틱 클릭 | 시점과 확대율 초기화 |
| 잡기 버튼 + 스틱 | 시점 회전 |

[개발 관례](CLAUDE.md) · [Noto Sans KR 라이선스](assets/OFL-NotoSansKR.txt)

플레이어 소스 코드에는 WTFPL v2가 적용됩니다. [라이선스 전문](LICENSE)
