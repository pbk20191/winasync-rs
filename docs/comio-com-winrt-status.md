# comio COM/WinRT 노출 현황

*기준일: 2026-11-09*

## 방향 (2026-11-09 결정)

**최종 목표는 WinRT 개념으로 스크립트 월드까지 노출하는 것**이고, 타깃 스크립트 호스트는 **모던 쪽(Python / Node / .NET 스크립팅)만**이다. 활성화는 당분간 HKLM(개발 중 승격 허용), 배포 모델은 나중에 정한다.

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
| 메타데이터 | 17개 타입(클래스 8 + 인터페이스 9)의 단일 출처 | `metadata/rustwinui3loop.rdl` |
| 생성기 | RDL → `.winmd` → Rust 바인딩. 손으로 돌린다 | `tools/winmdgen` (`cargo run -p winmdgen`) |
| 산출물 | 소비자가 프로젝션을 만들 때 쓰는 파일 | `metadata/rustwinui3loop.winmd` |
| 바인딩 | `_Impl` 트레이트 + vtable. 체크인됨 | `src/comio/bindings.rs` |
| 구현 | 위 트레이트를 구현 | `listener.rs` / `connector.rs` / `pipe.rs` / `unix.rs` / `hvsock.rs` / `stream.rs` |
| 활성화 | CLSID로 `CoCreateInstance`(HKCU) + 런타임 클래스 이름으로 `RoActivateInstance`(HKLM) | `activation.rs` |

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
regsvr32 /s rustwinui3loop.dll
TlbImp.exe "%LOCALAPPDATA%
ustwinui3loop
ustwinui3loop.tlb" ^
    /out:Rustwinui3loop.Interop.dll /namespace:Rustwinui3loop
```

```powershell
Add-Type -Path .\Rustwinui3loop.Interop.dll
$l = New-Object Rustwinui3loop.NamedPipeConnectionListenerClass   # -ComObject 아님
Register-ObjectEvent -InputObject $l -EventName ConnectionReceived -SourceIdentifier conn -Action {
    $EventArgs.ReadText(64)
}
$l.Bind($name)
```

**같이 사라진 것 하나 — `CoIncrementMTAUsage`.** `SinkSet::advise`가 첫 `Advise` 때 불러서 프로세스에 MTA가 살아 있게 했다. 아래 "측정으로 밝혀진 것"의 `CO_E_NOTINITIALIZED`가 그 이야기인데, 지금은 아파트먼트를 넘는 콜백 경로가 없어서 필요가 없다. `.winmd` 이벤트 경로가 아파트먼트를 다시 넘게 되면 **같은 문제가 같은 모양으로 돌아온다**. 그때 다시 만들 것.

**낡은 등록.** 이전 빌드로 등록한 적이 있으면 `Interface\{…}`와 `TypeLib` 키가 레지스트리에 남는다. 지금 바이너리는 그 IID들을 모르므로 지우지 못한다. 무해하다 — 옛 바이너리로 `regsvr32 /u` 하거나 그냥 둬도 된다.

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
| `GetRuntimeClassName` | `"Rustwinui3loop.<클래스>"` | `#[implement]` 목록 **첫 타입**의 `RuntimeName::NAME` | **있다** — 첫 줄을 런타임 클래스 타입으로 바꾸면 끝 |
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

[비패키지 서드파티 WinRT](https://blogs.windows.com/windowsdeveloper/2019/04/30/enhancing-non-packaged-desktop-apps-using-windows-runtime-components/)의 활성화는 `HKLM\SOFTWARE\Microsoft\WindowsRuntime\ActivatableClassId`(머신 전역, 승격 필요) 아니면 **소비하는 앱 쪽** `application.manifest`의 `<activatableClass>`(reg-free WinRT)다. 후자는 `python.exe` / `node.exe` 같은 공유 인터프리터엔 못 넣는다. HKCU 경로는 패키지 앱용으로 보인다.

지금의 "승격 없는 `regsvr32` 한 줄"을 잃는다. 개발 중엔 승격을 받기로 했고, **배포 모델은 미정**이다.

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
   `IDispatch`를 지운 순간 ProgID 등록은 **거짓말이 됐다**. ProgID는 사실상 스크립트가 `New-Object -ComObject Rustwinui3loop.NamedPipeConnector` / `CreateObject(...)`라고 쓰기 위해 존재하는데, 지금 그렇게 하면 객체는 **만들어지고 메서드는 하나도 못 부른다**. 컴파일된 호출자는 CLSID를 쓰고 ProgID 키를 읽지 않는다. 키를 지웠고, `ComClass.progid`는 `ComClass.name`(런타임 클래스 이름)으로 개명했다 — 문자열 자체는 그대로다.

   테스트는 이제 **부재를 단언**한다. `CLSIDFromProgID`가 `CO_E_CLASSSTRING`(0x800401F3)을 돌려주는지 본다 — `REGDB_E_CLASSNOTREG`가 아니다. 그건 "CLSID는 아는데 등록이 안 됐다"는 뜻이고 여기선 이름 단계에서 먼저 실패한다. (처음에 틀리게 적었다가 테스트가 잡아냈다.)

   **CLSID / `InprocServer32` / `DllGetClassObject`는 남긴다.** HKCU `regsvr32`는 승격이 필요 없고 HKLM WinRT 등록은 필요하므로, 3번이 끝나고 PyWinRT로 검증되기 전까지 **이게 유일하게 동작하는 활성화 경로**다. 4번까지 통과하면 그때 다시 본다.

3. ~~HKLM `ActivatableClassId` 등록~~ · **완료** (2026-11-09, 승격 검증만 남음)
   클래스마다 `DllPath`(REG_SZ) + `ActivationType` / `Threading` / `TrustLevel`(DWORD). 값은 SDK의 `activationregistration.h`에서 가져왔다:

   | 값 | 뜻 | 주의 |
   | --- | --- | --- |
   | `ActivationType = 0` | `InProcess` | |
   | `Threading = 0` | `ThreadingType::BOTH` | **`ThreadingModel` 애트리뷰트 enum이 아니다.** 그쪽은 `Both = 3`이다. 헷갈려서 3을 쓰면 STA 전용 클래스로 등록된다 |
   | `TrustLevel = 0` | `BaseTrust` | `GetTrustLevel`이 답하는 값과 같다 |

   **HKCU는 안 된다 — 측정했다.** 같은 키를 `HKCU` 밑에 쓰고 `RoActivateInstance`를 부르면 `REGDB_E_CLASSNOTREG`다. 비패키지 컴포넌트의 per-user 경로는 없다. 그래서 HKLM이고, 승격이 필요하다.

   `DllRegisterServer`는 이제 **양쪽을 다 쓴다**. COM은 HKCU(승격 불필요), WinRT는 HKLM(승격 필요). 승격 없이 돌리면 HKLM이 거부되는데, 이때 **에러가 아니라 `S_FALSE`를 돌려준다** — `regsvr32`는 성공으로 보고 에러 대화상자를 안 띄우고, COM 활성화는 멀쩡히 동작하며, 반쯤 쓰인 상태도 남지 않는다. 승격하면 `S_OK`. 통합 테스트가 **돌아온 코드와 레지스트리 실제 상태가 일치하는지** 단언하므로 조용한 반쪽 등록은 통과하지 못한다.

   `DllUnregisterServer`는 반대로 **HKLM 거부를 무시**한다. 쓸 수 없는 하이브는 쓴 적도 없는 하이브이므로, 거부를 보고하면 COM 쪽을 정상적으로 지운 비승격 `regsvr32 /u`가 전부 실패처럼 보인다.

   **남은 것**: 승격해서 `RoActivateInstance`가 실제로 도는지 확인. 통합 테스트에 조건부로 들어 있다 — HKLM 키가 있으면 돌리고, 없으면 "skipped" 한 줄 찍는다.

4. **Python end-to-end 검증** · 중간 · **수용 테스트**
   PyWinRT로 프로젝션을 생성하고, Python 스크립트가 리스너를 만들어 `AcceptAsync`를 await 한다. 이게 되면 목표 달성이다.

5. **hvsock TLS** · 중간 · *독립 트랙*
   메모리에 "진짜 gap"으로 남은 유일한 항목(2026-11-08 보류). WinRT의 TLS는 `StreamSocket`의 `SocketProtectionLevel`에만 있고, hvsock은 **host↔guest 신뢰 경계**를 넘으므로 여기서만 실제 필요하다(로컬 파이프엔 무의미). Schannel을 `TransportStream` 위에 얹으면 `IInputStream`/`IOutputStream` 모양이 유지된다. `.winmd` 트랙과 겹치지 않으니 언제 해도 된다.

6. **out-of-process 활성화** · 중간 · *나중*
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
- HKLM `ActivatableClassId` 등록 (미커밋)

검증: `cargo test --all-features` 140 + 1 통과, `cargo doc` 경고 0, clippy 24건(전부 기존 `tp`/`rio`/`main.rs`).

### 재현

```powershell
# COM만 (HKCU, 승격 불필요) — DllRegisterServer가 S_FALSE를 돌려준다
Start-Process regsvr32 -ArgumentList '/s', 'target\debug\rustwinui3loop.dll' -Wait

# COM + WinRT (HKLM, 승격 필요) — S_OK, 이후 RoActivateInstance 동작
Start-Process regsvr32 -Verb RunAs -ArgumentList '/s', 'target\debug\rustwinui3loop.dll' -Wait

# 해제
Start-Process regsvr32 -Verb RunAs -ArgumentList '/s', '/u', 'target\debug\rustwinui3loop.dll' -Wait

# 전체 검증
cargo test --all-features
cargo clippy --all-targets --all-features
```

`DllRegisterServer`가 쓰는 것은 이제 CLSID 키뿐이다 — ProgID도, `.tlb`도, `Interface\{…}` 프록시 등록도 없다.
