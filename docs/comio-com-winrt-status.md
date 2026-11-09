# comio COM/WinRT 노출 현황

*기준일: 2026-11-09*

> **개명 (2026-11-09).** 크레이트 `rustwinui3loop` → **`winasync`**, WinRT 네임스페이스 `Rustwinui3loop` → **`Pbk20191.WinAsync.Comio`**. 아래 "개명" 절.

## 방향 (2026-11-09 결정)

**최종 목표는 WinRT 개념으로 스크립트 월드까지 노출하는 것**이고, 타깃 스크립트 호스트는 **모던 쪽(Python / Node / .NET 스크립팅)만**이다. 활성화는 **reg-free WinRT**(소비 프로세스의 매니페스트 / 액티베이션 컨텍스트)다 — 처음엔 HKLM + 승격으로 정했다가 **그 전제가 틀린 것으로 측정됐다**(아래 3번). 승격이 필요 없어졌다.

이 결정이 정하는 것:

- **`.winmd`가 메인 산출물이다.** 옵션이 아니다.
- **`.tlb` + IDispatch는 삭제했다** (2026-11-09). 처음엔 "동결"로 뒀다가 같은 날 지웠다 — 아래 "삭제한 것" 절.
- **클래식 스크립트 호스트는 범위 밖이다.** VBScript/WSH, VBA, Office 매크로는 WinRT보다 먼저 만들어졌고 동결돼 IDispatch만 안다. "레거시 COM을 버린다"와 "그 호스트들을 버린다"는 같은 문장이다.
- **폐기된 작업 둘**: 오퍼레이션 객체(상관 토큰), tlbimp interop 통합 테스트. 아래 "폐기" 절 참조.

수용 기준: **Python 스크립트가 리스너를 만들고 `AcceptAsync`를 await 한다.**

## 현재 상태

**`.winmd`가 생겼고, 인터페이스가 거기서 생성된다** (2026-11-09).

| 겹 | 무엇을 하나 | 위치 |
| --- | --- | --- |
| 메타데이터 | 17개 타입(클래스 8 + 인터페이스 9)의 단일 출처 | `metadata/Pbk20191.WinAsync.Comio.rdl` |
| 생성기 | RDL → `.winmd` → Rust 바인딩. 손으로 돌린다 | `tools/winmdgen` (`cargo run -p winmdgen`) |
| 산출물 | 소비자가 프로젝션을 만들 때 쓰는 파일 | `metadata/Pbk20191.WinAsync.Comio.winmd` |
| 바인딩 | `_Impl` 트레이트 + vtable. 체크인됨 | `src/comio/bindings.rs` |
| 구현 | 위 트레이트를 구현 | `listener.rs` / `connector.rs` / `pipe.rs` / `unix.rs` / `hvsock.rs` / `stream.rs` |
| 활성화 | 런타임 클래스 이름으로 `RoActivateInstance`(reg-free 매니페스트). CLSID `CoCreateInstance`는 소비자가 직접 등록하면 | `activation.rs` |

### 개명 (2026-11-09)

| | 전 | 후 |
| --- | --- | --- |
| cargo 패키지 / cdylib | `rustwinui3loop` | **`winasync`** |
| bin | `rustwinui3loop-demo` | `winasync-demo` |
| WinRT 네임스페이스 | `Rustwinui3loop` | **`Pbk20191.WinAsync.Comio`** |
| 공개 클래스 | `Rustwinui3loop.NamedPipeConnector` | `Pbk20191.WinAsync.Comio.NamedPipeConnector` |
| 메타데이터 파일 | `metadata/rustwinui3loop.{rdl,winmd}` | `metadata/Pbk20191.WinAsync.Comio.{rdl,winmd}` |

**왜 지금.** 네임스페이스는 사적인 이름이 아니라 **공개 식별자**다 — Python 사용자가 타이핑하는 것이고, 매니페스트 여섯 줄이 그 문자열이고, `#[guid(auto)]` IID 아홉 개가 **정규화된 이름의 해시**다. 바꾸면 IID가 전부 다시 유도된다. 외부 소비자가 0인 지금이 그게 공짜인 마지막 시점이었다. (CLSID 여섯 개는 손으로 박은 GUID라 영향 없다.)

**왜 이 이름.** 옛 이름은 내용물을 가리키지 못했다 — comio 7.8k / tp 7.0k / rio 3.2k / dq 1.5k 줄인데, `winui3`는 `src/dq/mod.rs` 한 군데뿐이고 `loop`는 네 기둥 중 아무것도 지칭하지 않는다. `winasync`는 네 기둥의 공통분모이고 crates.io에서 비어 있다 (`winio`는 피했다 — 이미 **우리 의존성**이다).

네임스페이스는 **벤더 + 프로젝트 + 모듈**이다. WinRT 관례는 역방향 DNS가 아니라 `Microsoft.UI.Xaml` / `Windows.Storage.Streams`처럼 벤더가 앞에 온다. 벤더 세그먼트는 작성자의 GitHub 정체성이라 레지스트리 없이도 충돌이 없다. 파일명도 네임스페이스와 같게 맞췄다 — 모든 WinRT 툴체인이 이름으로 메타데이터를 찾을 때 가정하는 규약이다.

**검증.** 양쪽 프로젝션을 새로 생성해서 끝까지 돌렸다 — Python `ROUND-TRIP-OK`(Hyper-V 포함), C# `CS-ROUND-TRIP-OK`.

**같이 고친 것.** `python/stage.sh`가 PyWinRT 패키지 경로를 하드코딩하고 있었는데(네임스페이스가 인코딩돼 있어서 개명하자 바로 깨졌다) `_table.pywinrt.txt`를 찾아서 유도하도록 바꿨다. `.winmd` 경로도 `$REPO`(Git Bash 형식 `/e/...`)로 넘기고 있었는데 pywinrt는 .NET 툴이라 해석하지 못하고, MSYS도 고쳐주지 않는다 — `<package>;<path>`가 한 인자이고 세미콜론이 path-list 구분자로 읽히기 때문이다. 상대 경로로 바꿨다. 셋째, 이전에 백슬래시가 반토막 나면서 깨져 있던 줄 연결도 복구했다.

### 왜 메타데이터가 먼저여야 했나

**손으로 쓴 `#[interface]`는 WinRT 인터페이스가 될 수 없다.** 측정해서 확인했다:

```rust
#[interface("…")] pub unsafe trait IFoo: IInspectable { … }
// error[E0405]: cannot find trait `IInspectable_Impl` in this scope
```

`windows-interface`는 자식 vtable을 `Parent_Vtbl::new::<Identity, OFFSET>`로 만들고 `Parent_Impl` 트레이트를 요구하는데, `IInspectable`은 둘 다 없다 (`new`가 타입 파라미터를 **셋** 받고, `IInspectable_Impl`은 존재하지 않는다). 그래서 손으로 쓴 인터페이스는 vtable 슬롯 3개 뒤에서 시작하고, 프로젝션은 6개 뒤를 기대한다. `windows-bindgen --implement`만 후자를 낸다:

```rust
base__: windows_core::IInspectable_Vtbl::new::<Identity, IConnectionReceivedArgs, OFFSET>()
```

즉 `.winmd`는 기존 표면을 **기술**하는 물건이 아니라 인터페이스가 거기서 **생성되는** 출처다. `connector.rs` / `listener.rs`의 `#[interface]` 선언 9개는 사라졌다.

### 확인된 것 (측정)

- **`windows-bindgen` 0.62.1이 0.100으로 만든 `.winmd`를 경고 없이 읽는다.** 버전 스큐 없음. `windows-rdl`/`windows-metadata`는 0.100에만 있지만 바이너리 `.winmd`를 낼 뿐이라 상관없다. 맞아야 하는 건 `windows-core`에 대고 코드를 생성하는 bindgen뿐이고, 0.62 라인이 있다.
- **`.winmd`가 독립 리더로 왕복한다.** `windows_rdl::writer()`로 되읽으면 이벤트·프로퍼티·제네릭 인스턴스화·`Activatable` 속성까지 그대로 복원된다.
- **IID는 유도된다.** midlrt 해시(UUIDv5)를 RDL이 계산한다 — MIDL 3.0과 같은 알고리즘이라 독립 소비자가 다시 계산해도 일치한다.

### IID가 전부 바뀌었다

필연이다. 새 인터페이스는 `IInspectable`을 상속하고 멤버 모양도 다르다. **다른 vtable에 옛 IID를 재사용하는 것은 COM에서 유일하게 용서받지 못하는 실수**이므로, "GUID frozen" 주석과 함께 옛 GUID는 폐기했다.

### 이름 정리 (2026-11-09)

메타데이터에 들어가는 순간 공개 표면이 되므로 지금 맞췄다.

| 전 | 후 | 이유 |
| --- | --- | --- |
| `OverlappedDuplexStream` | `TransportStream` | `overlapped`는 Win32 구현 세부사항이다 |
| `NamedPipeClient` | `NamedPipeConnector` | 세 커넥터가 세 가지 작명법이었다 |
| `UnixStreamSocket` | `UnixConnector` | 〃 — 리스너 쪽 `<Transport>ConnectionListener`와 짝이 맞는다 |
| `HvStreamSocket` | `HyperVConnector` | 〃 (`Hv` / `HyperV` 혼용도 정리) |

`src/comio/runtimeclass.rs`는 **삭제했다** — bindgen이 같은 모양의 클래스 타입을 직접 생성하므로 손으로 쓴 이름 캐리어가 필요 없어졌다.

### RDL에서 겪은 것 (문서에 없음)

1. `rdl.md`는 `#[activatable(1)]`로 적어 뒀지만 조회는 말 그대로 `{Name}Attribute`라 **`#[Activatable(1)]`**이어야 한다.
2. `use`는 `mod` 안이 아니라 **파일 스코프**에 둔다.
3. `///` 문서 주석은 `#[doc]`으로 파싱돼 깨진다. `//`를 쓴다.
4. **`--reference windows,skip-root,Windows`를 빠뜨리면 bindgen이 `TypedEventHandler`/`IAsyncOperation`을 언급한 멤버를 조용히 전부 버린다.** `winmdgen`에 assert를 걸어 뒀다.

### 구현이 짧아졌다

생성된 `_Impl` 트레이트가 `Result<T>`와 `&HSTRING`을 쓰므로 raw 포인터 out-param, `ManuallyDrop`, 손으로 쓴 `HRESULT`, null 체크가 전부 사라졌다.

```rust
fn ConnectAsync(&self, name: &HSTRING) -> Result<IAsyncOperation<TransportStream>>;
```

## 삭제한 것: IDispatch · 타입 라이브러리 · 커넥션 포인트 (2026-11-09)

`dispatch.rs` / `typelib.rs` / `connpt.rs` 세 파일과, 모든 `#[implement]` 목록의 `IDispatch` · `IConnectionPointContainer` · `IProvideClassInfo{,2}`, `.tlb` 생성·등록, `Win32_System_Ole`/`Win32_System_Variant` 피처가 함께 나갔다. 단위 테스트 11건, 통합 테스트 2건(`powershell_round_trip`, `wsh_event_round_trip`)도 같이 간다.

**왜.** 그 레이어가 기술할 수 있는 건 이 표면의 **블로킹 번역본**뿐이다. 오토메이션에는 파라미터화 인터페이스가 없어서 `IAsyncOperation<T>`는 타입 라이브러리에 **아예 등장할 수 없고**, 스크립트는 vtable이 await 가능한 자리에서 `Accept(timeoutMs)` + 큐를 받았다. 모든 멤버에 대해 모양이 다른 두 번째 기술을 들고 다니는 비용은 WinRT를 영영 소비할 수 없는 호스트를 위해서만 값을 한다. 그 호스트들은 범위 밖이라고 이미 정했으므로, 청중이 남아 있지 않았다.

**동작했었다.** 지운 게 안 되던 코드라서가 아니다. PowerShell/VBScript/WSH에서 전부 돌려보고 출력까지 확인했었다. 기록으로만 남긴다:

```text
regsvr32 /s winasync.dll
TlbImp.exe "%LOCALAPPDATA%\winasync\winasync.tlb" ^
    /out:WinAsync.Interop.dll /namespace:Pbk20191.WinAsync.Comio
```

```powershell
Add-Type -Path .\WinAsync.Interop.dll
$l = New-Object Pbk20191.WinAsync.Comio.NamedPipeConnectionListenerClass   # -ComObject 아님
Register-ObjectEvent -InputObject $l -EventName ConnectionReceived -SourceIdentifier conn -Action {
    $EventArgs.ReadText(64)
}
$l.Bind($name)
```

**같이 사라진 것 하나 — `CoIncrementMTAUsage`.** `SinkSet::advise`가 첫 `Advise` 때 불러서 프로세스에 MTA가 살아 있게 했다. 아래 "측정으로 밝혀진 것"의 `CO_E_NOTINITIALIZED`가 그 이야기인데, 지금은 아파트먼트를 넘는 콜백 경로가 없어서 필요가 없다. `.winmd` 이벤트 경로가 아파트먼트를 다시 넘게 되면 **같은 문제가 같은 모양으로 돌아온다**. 그때 다시 만들 것.

**낡은 등록.** 이전 빌드로 등록한 적이 있으면 `Interface\{…}`·`TypeLib`·`CLSID` 키가 레지스트리에 남는다. 지금 바이너리는 지우지 못한다 — 그 IID들을 모르고, 애초에 `DllUnregisterServer`가 없다(6번). 무해하다. 치우려면 옛 바이너리로 `regsvr32 /u` 하거나 `HKCU\Software\Classes\CLSID` 아래 해당 키를 직접 지우면 된다.

## 호스트별 도달 범위

| 호스트 | 객체 생성 | 메서드 호출 | 이벤트 | `IAsyncOperation` |
| --- | --- | --- | --- | --- |
| Rust | 직접 | 직접 | 직접 | **직접** |
| C++ | 됨 (`CoCreateInstance` + vtable) | 됨 | 됨 (`TypedEventHandler`) | 수동 선언하면 |
| C# / .NET | CLSID로는 됨 | 인터페이스 재선언 필요 | 재선언 필요 | `.winmd` 전엔 불가 |
| Python / Node | **불가** | — | — | — |
| WSH / VBScript / VBA | **불가** (2026-11-09 이전엔 됐다) | — | — | — |

목표는 4번째 줄을 `.winmd` + 프로젝션으로 전부 "직접"으로 바꾸는 것이다. 5번째 줄은 돌아오지 않는다.

## 측정으로 밝혀진 것

세 건 모두 **in-process 테스트는 전부 통과하는데 외부 호스트에서만 깨지는** 종류였다.

### `CO_E_NOTINITIALIZED` — 싱크 호출이 조용히 실패

스레드풀 스레드는 `CoInitializeEx`가 불린 적이 없다. 이런 스레드는 **암묵적 MTA**가 서비스하는데, 그건 MTA가 존재할 때만이다. `cscript`처럼 호스트 스레드가 전부 STA면 프로세스에 MTA가 없어 첫 `Invoke`가 실패하고 이벤트가 사라진다.

해법은 첫 `Advise` 때의 `CoIncrementMTAUsage` (`connpt.rs`). 이건 **스레드를 초기화하지 않고** 프로세스에 MTA가 살아있게만 한다. 스레드풀 스레드에 `CoInitializeEx`를 부르면 안 되는 이유:

- 콜백 끝에 `CoUninitialize`했다가 하필 마지막 MTA 참조였다면 **프로세스 전체의 마샬링된 프록시가 죽는다**.
- 해제하지 않으면 남의 콜백을 실행할 공유 스레드의 아파트먼트 상태를 영구히 바꿔놓는 것이다.

> MS 문서의 "WinRT를 쓰면 `CoInitializeEx` 대신 `Windows::Foundation::Initialize`"는 **내가 소유한 스레드를 초기화할 때**의 이야기다. 우리 콜백 스레드는 WinRT 클래스를 **활성화하지 않고** 이미 존재하는 인터페이스 포인터로 메서드만 부르므로, 아파트먼트만 필요하고 WinRT 초기화는 불필요하다. (`listener.rs:42` 참조 — 객체는 Rust 팩토리에서 오지 `RoActivateInstance`가 아니다.)

> 이 수정(`connpt.rs`의 `CoIncrementMTAUsage`)은 그 파일과 함께 삭제됐다. 교훈만 남긴다 — `.winmd` 이벤트 경로가 아파트먼트를 다시 넘는 순간 똑같이 필요해진다.

### `TYPE_E_LIBNOTREGISTERED` — 타입 라이브러리 LCID *(삭제된 레이어)*

`CreateTypeLib2`가 라이브러리 LCID를 **사용자 기본값**(여기선 412, ko-KR)으로 잡아 그 아래 등록했고, `LoadRegTypeLib`는 중립 0으로 찾아 못 찾았다. `SetLcid(0)`으로 고정했었다. 언어 설정이 다른 장비에서만 재현되었을 버그다. **일반적인 교훈**: 사용자 로케일을 기본값으로 읽는 Win32 API는 개발 장비가 en-US면 영영 안 터진다.

### PowerShell 이벤트 — 틀리게 문서화했던 것 *(삭제된 레이어)*

"PowerShell은 이 이벤트를 받을 수 없다"고 적었는데, 그건 **타입 라이브러리를 만들기 전 기준**이었다. `.tlb`를 추가한 순간 tlbimp이 필요한 글루를 전부 생성할 수 있게 됐는데, 그걸 다시 확인하지 않았다. **일반적인 교훈**: 측정해서 적은 사실에는 측정 시점이 붙어 있다. 전제를 바꾸는 커밋은 그 사실을 다시 재야 한다.

## `IInspectable` — 이름은 풀렸다 (2026-11-09), `GetIids`는 남았다

우리 객체는 진짜로 `IInspectable`이다 — `IClosable`, `IInputStream`, `IOutputStream`이 전부 WinRT 인터페이스라 QI가 성공한다. 세 메서드 중 **이름은 고쳤고**, `GetIids`는 그대로 비어 있다.

| 메서드 | 지금 응답 | 어디서 결정되나 | 훅 |
| --- | --- | --- | --- |
| `GetRuntimeClassName` | `"Pbk20191.WinAsync.Comio.<클래스>"` | `#[implement]` 목록 **첫 타입**의 `RuntimeName::NAME` | **있다** — 첫 줄을 런타임 클래스 타입으로 바꾸면 끝 |
| `GetIids` | `count=0`, `values=null` | `IInspectable_Vtbl::new`가 설치하는 **자유 함수**에 하드코딩 | **없음** — 타입 파라미터를 아예 안 받음 |
| `GetTrustLevel` | `0` (BaseTrust) | 매크로가 `TrustLevel=` 속성값을 직접 넣음 | 있다 (`Partial`/`Full`). `BaseTrust`가 맞는 답이라 안 쓴다 |

### 이름을 어떻게 고쳤나 — `comio::runtimeclass`

훅은 좁고 문서화돼 있지 않다. `IInspectable_Vtbl::new::<Identity, Name, OFFSET>`이 `GetRuntimeClassName::<Name>`을 설치하고, `#[implement]`가 **목록의 첫 타입**을 그 `Name`으로 넘긴다 (`windows-implement-0.60.2/src/gen.rs:142`, 주석이 그렇게 적혀 있다).

전에 첫 줄은 늘 우리 `#[interface]` 트레이트였고, `#[interface]`는 `impl RuntimeName for X {}` — 기본값 `""` — 를 자동 생성한다 (`windows-interface-0.59.3/src/lib.rs:110`). 그래서 빈 문자열이었다. 유일한 예외가 `OverlappedDuplexStream`으로, 첫 줄이 `IInputStream`이라 `"Windows.Storage.Streams.IInputStream"`을 돌려주고 있었다 — 빈 문자열보다 나쁘다. MS **인터페이스** 이름을 자기 **클래스** 이름이라고 답한 것이다.

`src/comio/runtimeclass.rs`가 클래스마다 타입 하나를 둔다. `windows-bindgen`이 런타임 클래스를 뽑아내는 모양 그대로다 — `IUnknown`의 transparent 래퍼에, `Interface::Vtable`/`IID`는 그 클래스의 **디폴트 인터페이스**에서 그대로 빌리고, `RuntimeName`만 자기가 갖는다. `windows-0.62.2`의 `Windows::Storage::Streams::Buffer`와 비교하면 똑같다. 이걸 `#[implement]` 첫 줄에 넣으면 COM이 보는 vtable은 **하나도 안 바뀌고**(그 vtable이 곧 디폴트 인터페이스니까) 이름만 사실이 된다.

이름은 전부 이미 등록돼 있던 ProgID 문자열 그대로다. 거꾸로 `activation::CLASSES`가 `progid`를 **여기서 읽어간다**(`class_name::<T>()`). COM 이름과 WinRT 이름이 같은 문자열임이 구조적으로 보장되고, 한쪽만 리네임되는 일이 불가능하다.

비활성 클래스 둘(`ConnectionReceivedArgs`, `OverlappedDuplexStream`)도 이름을 받았다. 핸들러가 받아든 객체가 자기가 뭔지 답할 수 있어야 하는 건 마찬가지다.

### 이름을 안 준 것

`asyncop::IoAsyncOp`와 `connector`의 `ConnectOp`. 첫 인터페이스가 **제네릭** WinRT 인터페이스인데 `RuntimeName::NAME`은 타입 인자를 실을 수 없는 `&'static str`이다 (`windows-core-0.62.2/src/runtime_name.rs:3`의 TODO가 바로 이 이야기다). 프로젝션은 async op을 이름이 아니라 `IAsyncInfo` + 오퍼레이션 인터페이스로 찾으니, 여기서는 빈 문자열이 정직한 답이다.

### `GetIids`는 왜 못 고치나

`IInspectable_Vtbl::new`가 설치하는 `GetIids`는 타입 파라미터가 **하나도 없는** 자유 함수다. 이름과 달리 비집고 들어갈 틈 자체가 없다. 고치려면 `#[implement]`가 생성하는 identity vtable을 손으로 쓰는 수밖에 없다. `windows-implement` 0.100도 구조가 같아 업그레이드로 풀릴 문제가 아니다.

### 이름이 정직해진 것 ≠ 런타임 클래스가 된 것

런타임 클래스 이름은 "이 이름이 WinRT 메타데이터로 **해석된다**"는 약속이기도 하다. `.winmd`를 아직 안 내놓으니 `RoGetActivationFactory`는 여전히 실패한다. 그러니 지금 이름은 **메타데이터가 지켜야 할 약속을 코드에 먼저 적어둔 것**이다 — 생성기가 읽을 단일 출처가 생겼고, 두 번째 목록을 따로 관리할 필요가 없어졌다.

### 못 박아 둔 것

`comio::tests::iinspectable_is_shaped_but_hollow` — characterisation 테스트. 이름은 이제 `runtimeclass`가 정의한 상수와 **정확히 일치**하는지 비교하고(ProgID와 같은 상수라 한쪽만 바뀌면 깨진다), `GetIids`/`GetTrustLevel`은 여전히 현재 응답을 못 박는다. **컨트롤**로 진짜 WinRT 클래스(`Windows.Storage.Streams.Buffer`)를 같은 코드 경로로 통과시켜, 빈 `GetIids`가 호출 방식 탓이 아니라 객체의 답임을 보인다.

## `.winmd`로 가는 길

방향은 정해졌으므로 아래는 비교가 아니라 **무엇이 걸려 있는지**다.

### 얻는 것

비동기 모델을 **번역하지 않고 그대로 기술**한다. `IAsyncOperation<T>`가 네이티브로 나오고, `Completed`가 콜백이고, `Cancel`과 `ErrorCode`가 딸려온다. 커넥션 포인트도 블로킹 투영도 필요 없다. 컴파일된 소비자에게 `.tlb`가 **번역**인 데 반해 `.winmd`는 **기술**이다.

소비 경로는 프로젝션 생성이다. [PyWinRT](https://github.com/pywinrt/pywinrt)는 `pywinrt.exe`로 우리 `.winmd`에서 Python 프로젝션을 생성하고, 생성된 패키지의 `__init__.py` 옆에 우리 DLL을 둔다. [CsWinRT](https://github.com/microsoft/CsWinRT/blob/master/docs/authoring.md)도 같은 모양이다.

### 비용 1 — 활성화가 HKLM으로 간다

*(이 절은 2026-11-09 측정 전에 쓴 것이고, 결론이 틀렸다 — 3번 참조.)* [비패키지 서드파티 WinRT](https://blogs.windows.com/windowsdeveloper/2019/04/30/enhancing-non-packaged-desktop-apps-using-windows-runtime-components/)의 활성화는 `HKLM\...\ActivatableClassId` 아니면 reg-free 매니페스트다. HKLM은 **승격으로도 못 쓰고**(TrustedInstaller 소유), 매니페스트는 `python.exe`에 못 넣지만 **런타임 `CreateActCtx`로 올리면 된다** — 그래서 reg-free가 답이다.

→ **잃지 않는다.** reg-free는 승격도 레지스트리도 쓰지 않는다. 배포는 매니페스트 한 장을 DLL 옆에 두는 것이 전부다.

### 비용 2 — 표면의 기술을 어디에 둘 것인가 (미결)

순수 Rust로 가능하다 — `windows-metadata`는 ECMA-335를 읽고 **쓰며**, `windows-rdl`이 RDL → `.winmd` 컴파일을 한다. midl/cl.exe 불필요. 그러니 진짜 작업량은 라이터가 아니다.

우리 dispatch 테이블은 **블로킹 dispinterface 모양**을 기술하는데, `.winmd`가 기술해야 하는 건 **비동기 vtable 모양**(`IConnectionListener`, `IAsyncOperation` 반환)이고 그건 Rust `#[interface]` 선언에만 존재한다. 테이블에서 자동 생성이 안 되고, 그 표면에 대한 **새 기술**이 필요하며, `.tlb`에서 구조적으로 막아둔 드리프트 위험이 거기서 다시 들어온다.

선택지: RDL 파일을 소스 오브 트루스로 두거나, `windows-metadata` 라이터로 테이블에서 직접 방출하거나.

### 비용 3 — `IInspectable` (대부분 해소됨, 2026-11-09)

여기 "`GetRuntimeClassName`은 `#[implement]`로는 불가능하다"고 적혀 있었다. **틀렸다.** 훅이 있었고 (`#[implement]` 목록의 첫 타입이 `RuntimeName`을 공급한다), 위 절에서 그걸 썼다. 이 방향의 블로커였던 것이 블로커가 아니다.

남은 건 `GetIids`뿐인데, 이건 프로젝션이 요구하지 않는다 — 타입 해석은 이름으로 한다. 요구하는 소비자가 실제로 나타나면 그때 identity vtable을 손으로 쓰면 된다. **`.winmd` 트랙의 선행 조건이 아니다.**

## 다음 작업

`.winmd` 트랙이 주 경로다. 이름이 풀렸으므로 1번은 더 이상 "할 수 있나"가 아니라 "메타데이터가 코드의 이름과 맞물리나"의 확인이다.

1. ~~최소 `.winmd`로 확인~~ · **완료** — RDL 트랙으로 정했고 전체 표면을 기술했다.

2. ~~`DllGetActivationFactory` + `IActivationFactory`~~ · **완료** (2026-11-09)
   `DllGetClassObject` 옆에 export 하나. `IActivationFactory::ActivateInstance`는 IID도 aggregation outer도 받지 않고 `IInspectable`을 그냥 돌려주므로 `IClassFactory`보다 단순하다. 여섯 클래스 전부 기본 생성 가능해서 `IXxxFactory`는 필요 없다. 반환값은 문서화된 계약 그대로 — 인자가 null이면 `E_INVALIDARG`, 모르는 클래스면 `REGDB_E_CLASSNOTREG`.

   `class_for_name`이 `CLASSES`를 런타임 클래스 이름으로 찾는다. ProgID와 같은 상수이므로 **COM 활성화와 WinRT 활성화가 같은 테이블 하나를 읽는다.**

   통합 테스트가 `RoActivateInstance`가 아니라 **export를 직접 호출**한다 — HKLM 등록(아래 3번)이 아직 없기 때문이고, 그 덕에 "팩토리가 동작한다"와 "레지스트리가 우리를 가리킨다"가 분리된다. 활성화된 객체에게 `GetRuntimeClassName`을 다시 물어 **요청한 이름과 일치하는지** 확인하므로, 한쪽만 리네임되면 깨진다. 비활성 클래스(`TransportStream`, `ConnectionReceivedArgs`)와 모르는 이름은 `REGDB_E_CLASSNOTREG`로 거절되는 것까지 확인한다.

2b. ~~ProgID 키 제거~~ · **완료** (2026-11-09)
   `IDispatch`를 지운 순간 ProgID 등록은 **거짓말이 됐다**. ProgID는 사실상 스크립트가 `New-Object -ComObject Pbk20191.WinAsync.Comio.NamedPipeConnector` / `CreateObject(...)`라고 쓰기 위해 존재하는데, 지금 그렇게 하면 객체는 **만들어지고 메서드는 하나도 못 부른다**. 컴파일된 호출자는 CLSID를 쓰고 ProgID 키를 읽지 않는다. 키를 지웠고, `ComClass.progid`는 `ComClass.name`(런타임 클래스 이름)으로 개명했다 — 문자열 자체는 그대로다.

   테스트는 이제 **부재를 단언**한다. `CLSIDFromProgID`가 `CO_E_CLASSSTRING`(0x800401F3)을 돌려주는지 본다 — `REGDB_E_CLASSNOTREG`가 아니다. 그건 "CLSID는 아는데 등록이 안 됐다"는 뜻이고 여기선 이름 단계에서 먼저 실패한다. (처음에 틀리게 적었다가 테스트가 잡아냈다.)

   **CLSID / `InprocServer32` / `DllGetClassObject`는 남긴다.** 컴파일된 COM 호출자에게 여전히 유효하다. (3번에서 reg-free가 동작하는 것이 확인됐으므로 "유일하게 동작하는 경로"라는 이유는 사라졌다 — 4번 뒤에 다시 본다.)

3. ~~활성화 등록~~ · **완료** (2026-11-09) — **단, HKLM이 아니라 reg-free로**

   처음엔 HKLM `ActivatableClassId`로 계획했고 "개발 중엔 승격 OK"로 정해 뒀다. **그 전제가 틀렸다.** 승격해도 못 쓴다:

   ```
   Owner: NT SERVICE\TrustedInstaller
   NT SERVICE\TrustedInstaller   Allow  FullControl
   BUILTIN\Administrators        Allow  ReadKey      ← 읽기만
   ```

   승격한 관리자 토큰으로 `DllRegisterServer`를 직접 불러 확인했다 — `S_FALSE`, 아무것도 안 써진다. 이 머신에 등록된 **4,819개 클래스 전부가 `C:\Windows` 안**이고 서드파티는 **0개**다. 비패키지 서드파티에게 이 경로는 닫혀 있다.

   **답은 reg-free WinRT다.** 소비 프로세스가 매니페스트를 들고 있으면 된다 — 자기 애플리케이션 매니페스트로 넣거나, 런타임에 `CreateActCtx` + `ActivateActCtx`로 올리거나. 후자가 `python.exe`처럼 매니페스트를 건드릴 수 없는 호스트의 답이다. 측정 결과:

   ```
   without act ctx: Pbk20191.WinAsync.Comio.NamedPipeConnector -> HRESULT(0x80040154)
   with    act ctx: Pbk20191.WinAsync.Comio.NamedPipeConnector -> OK  name="Pbk20191.WinAsync.Comio.NamedPipeConnector"
   ```

   **지금까지 만든 건 하나도 안 버린다.** reg-free도 같은 `DllGetActivationFactory`를 부르고 같은 메타데이터를 읽는다 — 바뀌는 건 *발견 방법*뿐이다.

   `activation::manifest()`가 `CLASSES`에서 매니페스트를 생성한다(이름이 어긋날 수 없게). 통합 테스트가 **승격 없이 전 구간을 돈다**: 액티베이션 컨텍스트 *없이* 먼저 실패하는지 확인(컨트롤)하고, 올린 뒤 활성화해서 `GetRuntimeClassName`과 `IStreamConnector`까지 본다.

   ~~`register_winrt_under`는 남긴다~~ — **지웠다. 아래 6번 참조.**

4. ~~Python end-to-end 검증~~ · **완료** (2026-11-09) — **수용 기준 달성**

   ```text
   listening on \.\pipe\comio_py_29588
   connector transport = namedpipe
   client connected
   accepted from '\.\pipe\comio_py_29588'
   server read 'hello from python'
   ROUND-TRIP-OK
   ```

   Python 스크립트가 **레지스트리도 승격도 C++ 빌드도 없이** 우리 Rust 객체를 WinRT 프로젝션으로 돌렸다. `python/` 참조.

   - `await listener.bind_async(...)` — 진짜 `IAsyncAction`을 asyncio가 await
   - `add_connection_received(...)` — Rust 스레드풀 스레드가 Python 함수를 호출, `call_soon_threadsafe`로 루프 복귀
   - `await connector.connect_async(...)` — 진짜 `IAsyncOperation<TransportStream>`
   - `DataWriter`/`DataReader` — **윈도우 자신의** 스트림 클래스가 우리 스트림을 읽고 썼다. `TransportStream`이 진짜 `IInputStream`/`IOutputStream`이 아니면 불가능하다.

   **PyWinRT는 우리 winmd를 그대로 먹었다.** 걱정했던 `[ContractVersion]`/`[Version]` 부재는 문제가 아니었다. 8 클래스 + 9 인터페이스 전부 생성됐고, 파라미터화 IID도 정확하다 — `AsyncOperationCompletedHandler<TransportStream>`의 시그니처가 `rc(Pbk20191.WinAsync.Comio.TransportStream;{905a0fe2-...})`로 나왔는데 저 GUID가 우리가 선언한 디폴트 인터페이스 `IInputStream`이다.

   ### 걸린 것 1 — `windows-rdl`은 클래스 생성자를 못 낸다

   `windows-rdl`의 클래스 인코더는 TypeDef + InterfaceImpl + 애트리뷰트만 쓰고 **MethodDef를 하나도 안 쓴다**(`reader/class.rs`). 그래서 클래스에 `.ctor` 행이 없고, PyWinRT는 `__new__` 없는 클래스를 낸다. 인스턴스 멤버는 멀쩡하다 — PyWinRT가 인터페이스에서(`declaring=`) 읽기 때문이다.

   애트리뷰트 문제가 **아니다**. 먼저 그쪽을 의심해서 SDK와 똑같이 `Activatable(65536, "...UniversalApiContract")` + `ContractVersion`까지 맞춰봤지만 변화 없었고, SDK의 `InMemoryRandomAccessStream`(팩토리 인터페이스 없는 activatable)이 `.ctor` 행만 있으면 된다는 것을 보여준다. 그 실험은 되돌렸다 — UniversalApiContract 소속을 주장하는 건 거짓이므로.

   우회는 `python/activate_instance.py`: 프로젝션이 이미 찾을 줄 아는 액티베이션 팩토리에 `IActivationFactory::ActivateInstance`를 직접 부른다.

   ### 걸린 것 2 — Hyper-V 멤버 → **해결** (시그니처 변경)

   ```text
   warning: the shape census has no callback for slot 6 with the ABI call shape
     ({u4u2u2u1u1u1u1u1u1u1u1}{u4u2u2u1u1u1u1u1u1u1u1}p)
   ```

   `(GUID, GUID) -> 포인터` 모양이 PyWinRT의 ABI 호출 모양 census에 없어서 `IHyperVConnector::ConnectAsync`와 `IHyperVListenerBind::BindEndpointAsync`가 투영은 되고 호출하면 raise했다. **하필 hvsock이 이 크레이트가 존재할 이유에 가장 가까운 전송**이라 그냥 둘 수 없었다 — host↔guest 신뢰 경계를 넘는 유일한 전송이고, WinRT가 이름조차 못 붙이는 유일한 전송이다.

   업스트림(testwinrt에 모양 추가)을 기다리는 대신 **메타데이터가 엔드포인트를 문자열로 받게 바꿨다**:

   ```python
   await listener.bind_service_name_async("vsock:54321")
   stream = await connector.connect_async("hv://loopback/vsock:54321")
   ```

   잃는 게 없다. `HvSocketAddr`의 `Display`/`FromStr`이 정확한 역함수이고(왕복 테스트로 고정), 양쪽 다 평범한 GUID로도 쓸 수 있다. Rust 호출자는 타입 있는 `&GUID` 진입점을 그대로 쓴다. 스크립트 입장에서도 raw GUID 두 개보다 `hv://loopback/vsock:5000`이 낫다.

   결과: **프로젝션이 경고 0으로 생성되고, Python에서 실제 AF_HYPERV 왕복이 돈다.**

   ```text
   hyper-v listening on hv://wildcard/vsock:54321
   hyper-v accepted
   hyper-v server read 'hello over hvsock'
   ```

   ### 파이프라인

   ```text
   metadata/*.rdl --winmdgen--> *.winmd --pywinrt--> _table.pywinrt.txt
                                                   --winrt.table--> _table.pywinrt
   ```
   PyWinRT는 텍스트 테이블을 내고 런타임은 바이너리를 읽는다. `winrt-table-compiler`(PyPI)가 그 사이를 잇는다 — 이게 없어서 한참 헤맸다.

5. ~~hvsock TLS~~ · **폐기** (2026-11-09) — 대신 **합성 가능하게** 만들었다

   근거를 두 번 느슨하게 잡았다가 물러섰다. 처음엔 "WinRT에 없는 gap"을 가치 근거로 썼는데 그건 **필요조건이지 충분조건이 아니다**. 다음엔 "컨피덴셜 VM이면 뒤집힌다"고 했는데, 거기서 필요한 건 **증명(attestation) 기반 키 수립**이지 TLS가 아니다 — 이미지에 심은 인증서는 그 게스트를 읽을 수 있으면 같이 읽힌다.

   **왜 불필요한가.** hvsock은 네트워크가 아니라 VMBus를 지난다. 호스트는 어차피 게스트 메모리를 읽고, 게스트끼리는 서로의 채널을 못 본다. 막아줄 중간자가 구조적으로 없다. MS 문서도 "VMBus 위로 직접 전송되므로 TLS 같은 표준 네트워크 암호화는 일반적으로 불필요 — 애플리케이션 고유의 규정 준수나 멀티테넌트 신뢰 경계가 요구하지 않는 한"이라고 적는다.

   **인가는 이미 더 나은 수단이 있다.** accept가 보고하는 peer `vm_id`는 커널의 진술이라 게스트가 위조할 수 없고, 이미 `RemoteEndpoint`로 올라온다. 다만 **파티션 단위이지 프로세스 단위가 아니다** — 게스트에는 AF_HYPERV용 프로세스 경계가 없어서, Winsock을 쓸 수 있는 게스트 프로세스면 등록된 service id에 닿을 수 있다. 호스트 쪽은 HCS의 `BindSecurityDescriptor`/`ConnectSecurityDescriptor`로 프로세스 단위 제한이 가능하지만, 그건 VM을 구성하는 자리에서 설정하는 것이지 이 라이브러리가 할 일이 아니다.

   **그래서 구현 대신 합성 가능하게 바꿨다.** 아래 "ABI 다듬기" 2차 참조.

6. ~~자기등록 export~~ · **삭제** (2026-11-09) — ProgID와 같은 거짓말이었다

   export 다섯 개를 호출자 기준으로 다시 봤다.

   | export | 실제 호출자 | 판정 |
   | --- | --- | --- |
   | `DllGetActivationFactory` | `RoActivateInstance` → Python·C# | **유지** — `.winmd`를 의미 있게 만드는 유일한 export |
   | `DllGetClassObject` | `CoCreateInstance` | **유지** — 18줄, 아래 참조 |
   | `DllCanUnloadNow` | `CoFreeUnusedLibraries` | **삭제했다가 되살림** — 7번 |
   | `DllRegisterServer` | `regsvr32` | **삭제** |
   | `DllUnregisterServer` | `regsvr32 /u` | **삭제** |

   **`DllRegisterServer`가 왜 거짓말인가.** 3번에서 확인했듯 WinRT 절반은 **항상** `E_ACCESSDENIED`고 우리가 `S_FALSE`로 덮었다. 즉 성공처럼 보이는 호출이 실제로 남기는 건 HKCU CLSID 키뿐이다. 그런데 그 CLSID로 객체를 만들어도 **쓰려면 WinRT 타입이 필요하다** — `ITransportStream`은 `IInputStream`/`IOutputStream`/`IClosable`을 요구하고, 반환은 `IAsyncOperation<ITransportStream>`, 이벤트는 `TypedEventHandler<,>`다. 그걸 손으로 선언할 사람이면 이미 `.winmd`를 읽고 있고, 그러면 `RoGetActivationFactory`를 쓴다. 실제로 동작을 증명한 소비자(`python/`, `csharp/`)는 **둘 다 reg-free**다. CLSID 경로의 소비자는 자기 테스트가 유일했다 — 순환 논법이다.

   IDispatch를 지웠을 때 ProgID가 거짓이 된 것과 **같은 종류의 거짓이 한 단계 위에서** 벌어지고 있었다. 같은 이유로 같이 지웠다. (자기등록 자체도 MS가 10년째 인스톨러를 떼어내고 있는 방식이다 — MSIX와 reg-free가 대체재다.)

   **`DllCanUnloadNow`는 왜 같이 지우나.** 무조건 `S_FALSE`를 돌려주고 있었는데 — export를 **안 하면 동작이 똑같다**. `CoFreeUnusedLibraries`/`RoFreeUnusedLibraries`는 `GetProcAddress`로 찾고, 없는 모듈은 언로드를 시도하지 않는다. 거절을 부재가 이미 표현하고 있으니 함수는 의식(ceremony)이었다. *(이 판단은 같은 날 7번에서 뒤집혔다 — 무조건 거절이 유일한 답이라고 본 근거 하나가 틀렸다.)*

   **`DllGetClassObject`는 왜 남기나.** WinRT 인터페이스는 *COM 인터페이스이기도 하다.* CLSID를 **직접 등록한** 소비자는 액티베이션 컨텍스트 없이 `CoCreateInstance`로 같은 vtable을 받는다. 18줄이고, 지우는 건 능력을 없애는 것이지만 자기등록을 지우는 건 **"머신 상태를 우리가 관리한다"는 주장만 거둬들이는** 것이다. 컴포넌트는 이제 자기를 *기술*할 뿐 머신을 건드리지 않는다 — `manifest()`가 reg-free 매니페스트를 생성하고, `ComClass`(CLSID·이름·설명)와 `clsid_key()`가 인스톨러에게 필요한 걸 전부 공개한다.

   **증명은 그대로다.** 통합 테스트가 그 인스톨러 역할을 한다: 여섯 클래스 전부의 HKCU 키를 직접 쓰고, `CoCreateInstance`로 전 구간을 돌고, 어떻게 끝나든 `Drop`에서 지운다. 오히려 단순해졌다 — `DllRegisterServer`는 *자기가 실행되는 모듈*의 경로를 기록해서 반드시 DLL 안에서 돌아야 했는데(rlib 복사본을 부르면 테스트 exe가 등록된다), 호출자가 쓰면 경로는 그냥 인자다.

   남은 export는 정확히 둘이다:

   ```text
   1  DllGetActivationFactory
   2  DllGetClassObject
   ```

   Rust 테스트 142 + 1 통과, clippy 0, `cargo doc` 0.

7. ~~`DllCanUnloadNow`~~ · **되살림, 이번엔 진짜 답으로** (2026-11-09)

   6번에서 지운 근거는 둘이었다. 하나(*"`S_FALSE`는 부재가 이미 표현한다"*)는 맞다. 다른 하나 — **"정직하게 셀 방법이 없다"** — 는 틀렸고, 그 안의 구체적 주장이 틀렸다: *"같은 타입이 순수 Rust 경로에서도 만들어지는데 거긴 모듈 카운트를 건드릴 이유가 없다."* 뒤집혀 있다. Rust가 만든 객체도 **vtable이 이 모듈 안**이라 똑같이 세야 하고, rlib 빌드에서는 아무도 그 합을 읽지 않는다. **생성자에서 세면 두 빌드 모양 모두에서 맞다.**

   **무엇을 세나.** 셋인데 셋째를 빠뜨리기 쉽다.

   | | 무엇 | 어디 |
   | --- | --- | --- |
   | 객체 | `#[implement]` 타입 11개가 `ModuleRef` 필드를 든다 | `comio::unload` |
   | 서버 락 | `IClassFactory::LockServer(TRUE)` | 〃 |
   | **진행 중 I/O** | overlapped op — 완료 콜백이 이 모듈 안의 함수 포인터다 | `common::overlapped::LIVE_OPS` |

   셋째가 핵심이다. 살아 있는 COM 객체가 **하나도 없는데** op이 떠 있을 수 있고, 그 상태로 언로드하면 해제된 코드로 점프한다. 다행히 `LIVE_OPS`가 이미 정확히 그걸 세고 있었다 — 다만 `#[cfg(debug_assertions)]`라 릴리스에선 0에 고정이었다. **항상 켜도록 바꿨다**(op당 relaxed atomic 1회 — `StartThreadpoolIo` + syscall 옆에서는 잡음). 진단용이던 카운터가 안전성 근거가 됐으니 당연한 대가다.

   **구멍 하나를 메워야 했다.** `windows-future`가 만드는 객체는 vtable이 우리 DLL 안인데(Rust 의존성이라 여기로 모노모픽된다) **Drop 훅이 없다.** ABI를 넘는 곳이 셋이었고 전부 *이미 끝난* 결과라, 우리 타입으로 바꿨다 — `comio::settled`의 `SettledAction`/`SettledOp<T>`, 그리고 connect 실패 경로는 기존 `ConnectOp`를 즉시 완료시킨다. 비동기 동작은 일부러 시시하다; **타입을 소유하는 것**이 `ModuleRef`를 살 수 있는 유일한 길이라서 존재한다.

   **그리고 객체가 전부는 아니다.** 모듈이 설치한 **프로세스 전역 상태**도 돌려줘야 한다. `wsa_init()`이 첫 소켓에서 `WSAStartup`을 걸고 아무도 내리지 않았다. 그 상태로 언로드하면 참조가 영구히 남고, 다시 로드되면 새 모듈 이미지가 **또 `WSAStartup`**을 — 로드 주기마다 하나씩 쌓인다.

   기존 주석의 이유(*"라이브러리는 자기가 마지막 사용자인지 알 수 없다"*)는 API를 잘못 읽은 것이다. `WSAStartup`/`WSACleanup`은 refcount라서 **각자 자기 호출만 맞추면** 된다.

   **진짜 장애물은 `WSACleanup`이 아니라 포인터였다.** `unix.rs`/`hvsock.rs`가 `SIO_GET_EXTENSION_FUNCTION_POINTER`로 받은 `ConnectEx`/`AcceptEx` 주소를 `OnceLock`에 캐시하는데, 이건 **프로바이더 소유**다. refcount를 0으로 내리는 `WSACleanup`은 프로바이더를 내릴 수 있고, 그러면 리셋 불가능한 캐시가 해제된 코드를 가리킨다. 누수를 댕글링 호출과 바꾸는 건 방향이 거꾸로라, 한동안 `can_unload()`는 Winsock이 올라오면 그냥 "아니오"였다.

   **고쳤다 (2026-11-09).** `common::winsock::ExtFnCache`가 `OnceLock<usize>` 8개를 대체한다. 각 항목이 **채워질 당시의 teardown 횟수**를 같이 들고 있어서, `WSACleanup` 한 번이 전부를 한꺼번에 퇴역시키고 다음 조회가 재질의한다. 캐시를 무효화할 수 있게 되자 참조도 돌려줄 수 있게 됐다.

   | 조각 | 역할 |
   | --- | --- |
   | `STARTUP: Mutex<Startup>` | startup·teardown·**소켓 생성**을 서로 직렬화. 소켓 생성이 목록에 있는 게 핵심 — `wsa_init()` 후 별도 `WSASocketW`는 그 사이에 teardown이 끼어들 창을 남긴다. `winsock::socket()`이 둘을 잠금 아래 묶는다 |
   | `TEARDOWNS: AtomicUsize` | 캐시 항목의 도장. 잠금 없이 읽힌다 |
   | `winsock::pin()` | 회수 불가능한 프로바이더 상태를 쥔 모듈의 거부권. 유일한 호출자는 `rio::table` — RIO 함수 테이블을 `&'static`으로 내보내서 퇴역시킬 방법이 없다. rio는 `dylib` 빌드에 없으므로 실제로 막는 건 없다 |
   | `unload::release_process_state()` | `DllCanUnloadNow`가 `S_OK` 직전에 호출. **다른 데서는 안 부른다** |

   **왜 하필 거기인가.** 두 가지가 동시에 성립하는 유일한 지점이다 — ① `can_unload()`가 방금 "아무것도 안 살아 있음"을 확인했고, 우리 소켓은 전부 카운트되는 객체나 진행 중 op 안에 있으므로 **객체 0 + op 0 = 소켓 0**이다. ② `DllCanUnloadNow`는 `CoFreeUnusedLibraries`가 거는 평범한 호출이라 **로더 락 밖**이다. `DllMain`이었으면 `WSACleanup`이 프로바이더를 `FreeLibrary`할 수 있으니 바로 그 락이 막으려는 데드락이다. **`DllMain`은 관찰만 하고, 치우는 건 여기서 한다.**

   질의에 부작용이 있는 건 흔치 않지만, 호스트가 결국 언로드하지 않아도 무해하도록 골랐다: 다음 소켓 생성자가 `WSAStartup`을 다시 걸고, 캐시는 도장이 어긋나 전부 재질의한다.

   **App SDK는 치울 게 없었다** — `ensure_sdk`/`SdkGuard`가 `#[cfg(test)]`라 배포 DLL에 애초에 없다. 스레드풀 등록은 이미 카운트되는 객체가 `Drop`에서 닫는다.

   `tests/unload.rs`가 전 과정을 증명한다: 소켓 생성 → Winsock 올라옴 → 객체 해제 후 **언로드 가능**(예전엔 영구히 불가) → 해제 → 두 번째 해제는 아무것도 안 함 → **그리고 다시 연결이 된다**, 즉 낡은 `ConnectEx`로 호출하지 않는다.

   **측정.** `tests/unload.rs`는 통합 테스트다 — 카운터가 프로세스 전역이라 **자기 프로세스가 필요**하고, 단위 테스트였으면 객체를 만드는 다른 모든 테스트와 레이스한다. 증명 순서: 새 프로세스는 언로드 가능 → 객체 하나가 막고 해제하면 풀림(QI는 두 번째 이유가 아님) → 서버 락이 객체 없이도 막음(진짜 `IClassFactory` 경유) → 그리고 **객체 0개인데 op이 떠 있는 상태를 실제로 관찰**한다. 마지막 단언은 그 상태를 못 봤으면 실패한다 — `LIVE_OPS`가 다시 debug로 묶이면 바로 잡힌다.

   남은 레이스는 API에 내재한다. `can_unload()`는 스냅샷이고 COM이 받는 건 언제나 스냅샷이다: 마지막 객체의 `Drop`이 카운트를 0으로 내린 뒤에도 이 모듈 코드를 통해 반환해야 한다. MS 지침이 `DllCanUnloadNow`를 권고적으로 다루고 `CoFreeUnusedLibraries`가 언로드를 미루는 이유다. 우리가 보장하는 건 우리 몫의 절반이다 — **센 것이 하나라도 남아 있으면 `S_OK`는 절대 안 나간다.** 거짓 `S_OK`가 크래시고, 거짓 `S_FALSE`는 다음 스윕까지 상주할 뿐이다.

   **`dylib` 피처 뒤로 넣었다.** `crate-type = ["lib", "cdylib"]`은 rustc를 **한 번** 돌려 두 산출물을 내므로 "cdylib일 때만"을 가르는 cfg가 없다 — 명시적으로 선언하는 게 정직한 대체물이다. default에는 **안 넣는다**: 기본 빌드가 비용을 안 내는 게 요점이다.

   어디에 적용할지는 측정으로 갈렸다.

   - **`LIVE_OPS`** — `common::overlapped`는 comio·rio·tp **셋 다**의 제출 경로인데, 답할 `DllCanUnloadNow`가 있는 건 `dylib` 빌드뿐이다. `#[cfg(any(debug_assertions, feature = "dylib"))]` — 런타임 비용 0.
   - **`OUTSTANDING`** — 증가 지점 12곳이 전부 COM 객체 생성이다. 런타임 플래그로 가드하면 아끼려는 `fetch_add`와 **같은 값**(atomic 로드 + 분기)이 든다. 그래서 `ModuleRef`를 피처 없을 때 no-op ZST로 만들었다 — 필드 12곳은 그대로 두고 컴파일 타임에 사라진다.
   - **`DllGetClassObject` / `DllGetActivationFactory`는 게이트하지 않는다.** 차가운 진입점이고 op당 비용이 없다. 평범한 `cargo build`가 여전히 활성화되는 DLL을 내는 편이 낫다.

   `.def`는 build.rs가 생성한다 — export 목록이 피처에 달렸는데, 빌드가 정의하지 않는 심볼을 .def에 적으면 **export 누락이 아니라 링크 에러**(LNK2001)다. 실제로 6번에서 한 번 밟았다.

   **`DllMain`을 붙였다 — 치우려고가 아니라 잡으려고.** `DllCanUnloadNow`는 권고적이라 호스트가 안 물어보고 `FreeLibrary`해도 규칙 위반이 아니다. 그 상태로 언로드되면 실패는 **한참 뒤 전혀 다른 곳**에서 해제된 코드로 점프하는 모양으로 나타나고 여기로 역추적되지 않는다. detach 시점(`reserved`가 null = `FreeLibrary`, non-null = 프로세스 종료이고 후자는 문제가 아니다)에 debug 빌드에서 확인해 카운트를 찍고 `abort`한다. panic이 아니다 — 로더 락 아래 `extern "system"` 프레임에서 unwind는 보고하려는 버그보다 나쁘다.

   **치우는 일은 여기서 하지 않는다.** `DllMain`은 로더 락 아래서 돌고, 쓸모 있는 것들(`WSACleanup`, 스레드풀 콜백 대기, 모듈을 로드/언로드할 수 있는 모든 것)이 정확히 거기서 부르면 안 되는 것들이다. **관찰은 안전하고 행동은 아니다.** `DisableThreadLibraryCalls`도 안 부른다 — 스레드당 빈 호출 둘을 아끼자고 정적 TLS(`std` 경유로 이 모듈에 있다)와 나쁘게 얽힐 이유가 없다.

   `DllMain`이 export 테이블에 올라가는 건 막지 못했다. rustc는 cdylib에서 `no_mangle` 심볼을 **`pub` 여부와 무관하게** 전부 내보낸다. 대안인 `.CRT$XLB` TLS 콜백은 export되지 않고 같은 알림을 받지만, 링커가 `_tls_used`를 떨구면 **조용히** 안 돈다. 안 돌지도 모르는 진단보다 보이는 export가 낫다.

   **전역 상태 정리 (2026-11-09).** 위의 카운터들을 만들면서 크레이트 전체의 `static`을 전수 조사했다. 발견: **변경 가능한 전역은 거의 전부 테스트만 읽고 있었다.**

   | | 조치 |
   | --- | --- |
   | `tp::pool::CANCELED_CALLBACKS` | **삭제** — 쓰기만 하고 읽는 곳이 없었다 |
   | `dq::timer::NEXT_TIMER_ID` | **thread_local 안으로** — `TIMERS`가 thread_local인데 id는 프로세스 전역이라 **범위가 맵보다 넓었다.** 이제 같다 |
   | `LIVE_OPS` · `INLINE_COMPLETIONS` · `PUMP_RELEASES` | `#[cfg(any(test, feature = "counters"))]` — **static 자체가 존재하지 않는다**, 갱신만 끄는 게 아니라 |
   | `comio::unload::{OUTSTANDING, LOCKS, IN_DLL}` | `#[cfg(feature = "dylib")]` |
   | `winsock::WSA_UP` | 〃 — 읽는 곳이 `can_unload` 하나뿐이다 |

   `counters` 피처가 생겼고 `dylib`이 끌어온다. 기본은 꺼짐이고 **꺼짐이 요점이다**: 관측할 수 없는 카운터도 프로세스의 모든 소비자와 병렬 실행되는 모든 테스트가 공유하는 가변 상태다. 크레이트 자신의 단위 테스트는 `cfg(test)`로 켜지므로, 피처는 **릴리스 빌드에서 진단용으로 되켜는** 경우를 위해 있다.

   남겨둔 것과 이유:

   - **`PTR: OnceLock<usize>` 7개** + `rio::table::TABLE` — `WSAIoctl`로 받은 `ConnectEx`/`AcceptEx` 주소 캐시다. 캐시하는 대상 자체가 프로세스 전역(프로바이더 소유)이라 소켓마다로 내리면 **소켓당 ioctl 하나를 더 내고 얻는 게 없다** — `WSACleanup`을 할 게 아니라면. 그건 안 하기로 했다(위).
   - **`winsock::INIT: Once`** — `WSAStartup`이 정의상 프로세스 전역이다.
   - **`buffer::PAGE`, `dq::SDK`** — 각각 시스템 페이지 크기와 App SDK 부트스트랩. 초기화 후 불변이라 상태라기보다 사실이다.
   - **thread_local 4개**(`dq::CURRENT`, `DEDICATED_ANCHOR`, `TIMERS`, `rio::IN_PUMP`) — 전역이 아니라 **스레드별** 실행기 컨텍스트다. 없애려면 모든 API에 컨텍스트 인자를 꿰어야 한다.

   **그리고 구조로 강제했다 (2026-11-09).** `dylib` 피처는 "이 컴파일은 DLL이다"의 *대체물*이었다 — cdylib 여부를 볼 방법이 없어서. 확인해 보니 정말 없다: crate-type용 `cfg`는 rustc에 없고, `required-features`는 `[lib]`에 못 붙고, build.rs는 crate-type을 못 본다. 원인은 `crate-type = ["lib", "cdylib"]`이 rustc **한 번**으로 둘을 낸다는 것이라, **강제하는 유일한 방법은 그 모호함을 없애는 것**이었다.

   서버를 별도 패키지 `dll/`(`winasync-dll`)로 뺐다. crate-type이 `cdylib` **하나뿐**이라 이걸 컴파일하는 게 곧 서버 빌드다.

   | | 전 | 후 |
   | --- | --- | --- |
   | 루트 패키지 | `["lib", "cdylib"]` | rlib만. export·`DllMain`·`no_mangle` **하나도 없음** |
   | `Dll*` 진입점 | 라이브러리의 `extern "system"` | 라이브러리는 `pub fn dll_*` 본문만, `dll/`가 얇은 shim |
   | `dylib` 피처를 켜는 곳 | 기억해야 함 | `dll/`의 의존성 선언 **한 곳** |
   | DLL의 피처 | 테스트 하네스 cfg에서 재구성 | `comio` + `dylib` 고정 — 테스트하는 DLL이 곧 배포하는 DLL |
   | `.def` | 피처에 따라 build.rs가 생성 | 체크인된 `dll/exports.def` (export 목록이 이제 고정) |

   DLL 크레이트는 `windows-core` 하나만 의존한다 — shim이 `GUID`/`HRESULT`/`HSTRING`을 라이브러리 함수에 그대로 넘길 뿐이고, `IClassFactory`/`IActivationFactory`는 라이브러리 안에 남는다. lib 이름을 `winasync`로 둬서 산출물이 여전히 `winasync.dll`이고, 매니페스트의 `<file name>`과 양쪽 프로젝션이 그대로 동작한다. `tp`/`dq`/`rio`는 서버에 안 넣었다 — 컴포넌트는 `comio`이고, `rio`는 넣으면 Winsock teardown을 pin한다.

   **남는 틈 하나는 cargo의 것이다:** 한 빌드 안에서 피처는 통합된다. `cargo build --workspace`는 라이브러리를 `dylib` 켠 채로 한 번 컴파일해서 모든 멤버에 쓴다. 서버는 `-p winasync-dll`로, 소비자는 따로 빌드하면 각자 요청한 것을 받는다.

   덤으로 하나: `comio` **단독** 테스트 빌드가 원래부터 깨져 있었다. `tests/activation.rs`가 `CreateActCtxW`를 쓰는데 그 `windows` 피처를 `tp`/`dq`가 우연히 켜주고 있었다. 라이브러리는 그 API를 안 쓰므로 `comio`가 아니라 **dev-dependency**로 옮겼다.

   **`SetThreadpoolCallbackLibrary` (2026-11-09) — 그리고 앞선 감사에서 놓친 두 경로.** 이 API를 걸려면 comio가 스레드풀에 콜백을 거는 길을 전부 우리 환경으로 모아야 했고, 세어 보니 셋이었다: TP_IO 등록(환경 `NULL`), `listener.rs`의 `windows_threading::submit`, `pipe.rs`의 `IAsyncOperation::spawn`. 그 과정에서 **이전 감사가 틀렸던 게 드러났다.**

   - `NamedPipeConnector::ConnectAsync`가 `windows_future::IAsyncOperation::ready(...)`를 그대로 ABI 밖으로 내보내고 있었다. 앞서 "ABI를 넘는 windows-future 객체는 세 곳, 전부 교체"라고 했는데, grep이 `_Impl` 본문만 봤고 이건 `self.connect_async()`로 한 단계 위임돼 있어 빠졌다. 하필 Python·C#이 둘 다 쓰는 **가장 흔한 경로**라 성공한 파이프 연결마다 카운트 안 되는 객체가 하나씩 나갔다.
   - 대기 중·실행 중인 `submit` 작업 항목은 이 모듈 코드를 돌리는데 객체 카운트에도 `LIVE_OPS`에도 안 잡혔다.

   그래서 "센 것이 남아 있으면 `S_OK`는 안 나간다"는 참이었지만 "실행될 수 있는 건 전부 센다"는 **거짓이었다.**

   | 고친 것 | 방법 |
   | --- | --- |
   | 풀 진입점 | `comio::pool` 하나로. `submit`은 잡 안에 `ModuleRef`를 들고 가서 큐에 있는 동안·도는 동안 카운트된다. `windows-threading`은 comio 피처에서 빠졌다 |
   | 파이프 connect | `connector::finished` / `connector::spawned` — 둘 다 카운트되는 `ConnectOp` |
   | 환경 | `pool::callback_env()` — `RaceDll` = 우리 모듈. 모듈 핸들은 `DllMain` attach에서 받는다(`IN_DLL` 플래그가 핸들로 바뀜) |

   `tests/unload.rs`의 새 2b단계가 놓친 걸 잡는다: 파이프 connect는 객체를 **둘**(operation + 그 안의 stream) 늘려야 한다. 옛 코드로 되돌려 돌리면 정확히 그 단언에서 2 대신 1로 실패하는 것까지 확인했다.

   **`RaceDll`이 실제로 하는 일 — 측정해 보니 통념과 달랐다.** "I/O가 남아 있는 동안 DLL을 붙잡아 둔다"로 읽기 쉽고, `tp::pool`의 기존 문서도 그렇게 적혀 있었고, 나도 그렇게 썼다. 그 가정으로 테스트를 짰더니 debug `DllMain`이 detach에서 이렇게 잡았다:

   ```text
   FATAL: winasync unloaded with work outstanding -- 1 object(s), 0 server lock(s), 1 operation(s) in flight.
   ```

   release(detach가 abort하지 않음)에서 같은 시나리오를 `RaceDll` 유무만 바꿔 돌렸다:

   | | 읽기 대기 중 `FreeLibrary` | 나중에 읽기 완료 |
   | --- | --- | --- |
   | `RaceDll` 없음 | 언매핑 | **`STATUS_ACCESS_VIOLATION`**, 프로세스 사망 |
   | `RaceDll` 있음 | 언매핑 | **생존** — 풀이 사라진 모듈로 호출하지 않는다 |

   즉 **DLL을 붙잡지 않는다.** 이름 그대로 콜백과 언로드 사이의 *경합*을 정리할 뿐이다 — 사라진 모듈로는 호출을 거절하고 op 블록을 누수한다. 프로토콜을 이미 어긴 호스트에게는 맞는 거래다. 이미 *실행 중인* 콜백이 언로드를 만나면 반환까지 붙잡을 것으로 보이는데(경합 없이 거절하려면 호출을 가로질러 참조를 쥐어야 하므로), 그건 **추론이지 측정이 아니다**라고 문서에 구분해 적었다. 코드 주석 전부와 `tp::pool`의 `race_dll` 문서를 측정치로 고쳤다.

   `tests/racedll.rs`는 **release 전용**이다. debug DLL은 바로 이 호스트 행동을 버그로 보고 detach에서 abort한다 — 개발 중엔 시끄럽게, 배포에선 살아남게. 측정 중 한 번 헷갈린 것도 적어 둔다: 대조군 뒤 `mv`로 파일을 복구했더니 mtime이 빌드보다 오래돼 cargo가 재빌드하지 않았고, `RaceDll` 있는 코드인데 대조군 DLL로 돌아 크래시가 났다. `touch` 후 재빌드하니 3/3 통과.

   `winasync.dll`(`dll/`)의 export — 이제 피처와 무관하게 항상:

   ```text
   1  DllCanUnloadNow
   2  DllGetActivationFactory
   3  DllGetClassObject
   4  DllMain          (export될 이유는 없지만 막을 수 없다 — 위)
   ```

   라이브러리 단독 빌드는 DLL 자체를 만들지 않는다.

   *후속 (쉬움):* `LIVE_OPS`를 읽는 누수 단언들이 아직 `#[cfg(debug_assertions)]`다. 카운터가 정확해졌으니 이제 게이트를 풀면 릴리스에서도 의미가 생긴다 — tp/rio까지 번지는 별개 작업이라 미뤘다.

8. **out-of-process 활성화** · 중간 · *나중*
   WinRT에도 `OutOfProcessServer` 모델이 있다. 마샬링·아파트먼트·수명을 전부 강제로 통과시키는 값은 그대로지만, in-proc으로 3번을 먼저 끝내는 편이 낫다.

### 폐기 (2026-11-09 방향 결정에 따라)

- **오퍼레이션 객체(상관 토큰)** — IDispatch에서 비동기를 흉내 내려던 물건이다. WinRT 소비자는 `IAsyncOperation`을 그대로 받으므로 만들 이유가 사라졌다.
- **tlbimp interop 통합 테스트** — 레이어 자체가 삭제됐으니 자동으로 사라졌다. 레시피는 "삭제한 것" 절에 기록으로 남겼다.
- **`IAsyncOperation` 교차 아파트먼트 마샬링 측정** — `.winmd` 판단 근거로 필요했던 것인데 판단이 끝났다. 메타데이터를 내놓으면 자연히 답이 나온다.

### 가치 낮음

- **`DllCanUnloadNow` 객체 카운팅** — 지금 무조건 `S_FALSE`. 이유는 `activation.rs` 문서에 있고, 고치면 `CoFreeUnusedLibraries`가 동작. 위험 대비 이득 작음.
- ~~`EnumConnectionPoints` / `EnumConnections`~~ — 커넥션 포인트와 함께 삭제됐다.

### 위생

- 테스트 2곳의 `CoInitializeEx` → `RoInitialize`. 깨끗한 프로세스 3개로 재본 결과 **동작 차이는 없다**(초기화 없음 / `CoInitializeEx(MTA)` / `RoInitialize(MULTITHREADED)` 모두 WinRT 활성화·vtable 호출·`GetRuntimeClassName`·async op 동일). 버그 수정이 아니라 의도 표현이다. `Win32_System_WinRT` 피처가 필요하다.
- 기존 clippy 경고 24건 (`tp` / `rio` / `main.rs` / `listener.rs`). 이번 작업과 무관한 기존 것들.

## ABI 다듬기 (2026-11-09) — AF_UNIX 데이터그램을 미리 염두에 두고

지금은 전부 연결 지향이다. 만약 AF_UNIX **데이터그램**을 넣는다면(WinRT에 없는 진짜 gap — `DatagramSocket`은 IP 전용) WinRT 자신의 모델은 *bind + `MessageReceived` 이벤트 + `GetOutputStreamAsync(목적지)`*다. 그 모양을 옆에 놓고 보니 현재 ABI에 세 군데가 어색했고, **소비자가 없는 지금** 고쳤다. 의미 변화는 없는 순수 개명 + 비대칭 하나.

| 전 | 후 | 이유 |
| --- | --- | --- |
| `IStreamConnector` | `ITransport` | `Transport` 하나만 들고 있고 스트림적인 구석이 없다. 데이터그램 소켓도 같은 질문에 답한다 |
| (리스너는 미구현) | 리스너도 `ITransport` | **지금 있던 비대칭**이다 — 커넥터에겐 "어떤 전송이냐"를 물을 수 있는데 리스너에겐 못 물었다 |
| `I*ListenerBind` | `I*Bind` | 바인드는 역할이 아니라 주소 지정이다. 데이터그램 소켓은 `IUnixBind`를 그대로 재사용한다 |

리스너의 `Transport`는 커넥터와 **같은 상수**(`pipe::TRANSPORT` 등)에서 답한다 — 한 전송의 두 반쪽이 이름을 두고 엇갈릴 수 없게.

**`Information`은 일부러 안 뺐다.** WinRT 자신이 `StreamSocketListener`와 `DatagramSocket`에 각각 중복으로 둔다. 게다가 커넥터는 바인드하지 않으니 공용으로 빼면 빈 문자열을 돌려주는 거짓말이 된다.

### 이 과정에서 잡은 것

RDL에 `ITransport`를 추가하고 Rust `#[implement]`에는 안 넣었더니 **컴파일이 멀쩡히 통과했다.** 메타데이터는 "이 인터페이스를 구현한다"고 선언하는데 객체는 QI에서 거절하는 상태 — 소비자가 처음 시도할 때까지 아무도 모른다. `com_vtable_surface_works_for_every_transport`가 이제 `ITransport` QI를 단언한다.

## ABI 다듬기 2차 — 합성 가능하게 (2026-11-09)

TLS를 구현하지 않기로 했으면 **가져와서 감쌀 수 있어야** 한다. 그런데 메타데이터가 구체 클래스를 박아놔서 막혀 있었다 — 래퍼를 만들어도 우리 API를 통과할 수 없었다.

| 전 | 후 |
| --- | --- |
| (없음) | `interface ITransportStream: IInputStream + IOutputStream + IClosable` |
| `IConnectionReceivedArgs::Stream -> TransportStream` | `-> ITransportStream` |
| `I*Connector::ConnectAsync -> IAsyncOperation<TransportStream>` | `<ITransportStream>` |
| `TypedEventHandler<IConnectionListener, ConnectionReceivedArgs>` | `<..., IConnectionReceivedArgs>` |

이제 TLS 래퍼가 `ITransportStream`을 구현하면 **우리 것이 가는 모든 곳에 간다** — 커넥터 반환값, `IConnectionReceivedArgs`, `DataReader`. 이벤트 args도 인터페이스라 *합성된 리스너*(우리 걸로 accept하고 암호화된 스트림을 넘기는)가 이벤트를 띄울 수 있다. sender를 인터페이스로 둔 이유와 같은 논리다.

### 여기서도 같은 함정을 밟았다

`ITransportStream`이 디폴트 인터페이스가 되면서 `IInputStream`이 **어느 vtable 체인에도 없게 됐다** — 전에는 그게 디폴트라 클래스 항목이 공짜로 제공했다. 메타데이터는 여전히 `requires`로 약속하는데 QI는 거절하는 상태로, 컴파일은 멀쩡히 통과했다. **`requires`는 vtable 체인을 만들지 않는다.** `#[implement]` 목록에 직접 적어야 한다.

`a_stream_answers_everything_its_metadata_claims`가 이제 스트림이 약속한 인터페이스 전부에 답하는지 QI로 확인한다. 바로 앞의 `ITransport` 건과 같은 종류의 버그였고, 이번엔 테스트가 남는다.

## 커밋 상태

| 커밋 | 내용 |
| --- | --- |
| `eee9b30` | IDispatch 레이어 동결 표시 |
| `c2d5572` | 방향을 WinRT로, 폐기 항목 정리 |
| `ea0d115` | 이 문서 |
| `7aab8c7` | 타입 라이브러리 이후 서술 교정 + `IInspectable` 테스트 |
| `ace1f38` | 타입 라이브러리와 커넥션 포인트 |
| `c483249` | IDispatch |
| `abdb902` | 활성화 |
| `aa05a16` | 클라이언트 타입을 COM 커넥터로 |

**이번 세션 (2026-11-09)** — 중간 상태를 커밋해 두지 않아 앞 단계들은 하나로 합쳐 들어간다. 런타임 클래스 이름 작업(`runtimeclass.rs`)은 bindgen 출력으로 **대체되며 사라졌으므로** 따로 커밋할 산출물이 없고, IDispatch 삭제는 `.winmd` 전환과 같은 파일들을 건드려 분리하려면 `listener.rs` 800줄을 손으로 복원해야 한다:

- `ed43ce9` — IDispatch 레이어 삭제 + `.winmd` + 생성 바인딩 전환 + 클래스 리네임 + `DllGetActivationFactory`
- `17d6263` — ProgID 키 제거
- `1a3dc0e` — WinRT 활성화 등록 (HKLM 시도)
- `43a4750` — reg-free 매니페스트로 전환
- Python 수용 테스트 `python/` (미커밋)

검증: `cargo test --all-features` 140 + 1 통과, `cargo doc` 경고 0, clippy 24건(전부 기존 `tp`/`rio`/`main.rs`).

### 재현

```powershell
# COM 등록 (HKCU, 승격 불필요). DllRegisterServer는 S_FALSE를 돌려준다 —
# WinRT 키는 TrustedInstaller 소유라 못 쓰기 때문이고, 정상이다.
Start-Process regsvr32 -ArgumentList '/s', 'target\debug\winasync.dll' -Wait
Start-Process regsvr32 -ArgumentList '/s', '/u', 'target\debug\winasync.dll' -Wait

# WinRT는 등록이 없다. 소비자가 매니페스트를 올린다:
#   activation::manifest() -> winasync.manifest (DLL 옆)
#   CreateActCtx + ActivateActCtx -> RoActivateInstance
# 전체 검증
cargo test --all-features
cargo clippy --all-targets --all-features
```

`DllRegisterServer`가 쓰는 것은 이제 CLSID 키뿐이다 — ProgID도, `.tlb`도, `Interface\{…}` 프록시 등록도 없다.
