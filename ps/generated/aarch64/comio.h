/* Header file automatically generated from Pbk20191.WinAsync.Comio.idl */
/*
 * File built with Microsoft(R) MIDLRT Compiler Engine Version 10.00.0231 
 */

#pragma warning( disable: 4049 )  /* more than 64k source lines */

/* verify that the <rpcndr.h> version is high enough to compile this file*/
#ifndef __REQUIRED_RPCNDR_H_VERSION__
#define __REQUIRED_RPCNDR_H_VERSION__ 500
#endif

/* verify that the <rpcsal.h> version is high enough to compile this file*/
#ifndef __REQUIRED_RPCSAL_H_VERSION__
#define __REQUIRED_RPCSAL_H_VERSION__ 100
#endif

#include <rpc.h>
#include <rpcndr.h>

#ifndef __RPCNDR_H_VERSION__
#error this stub requires an updated version of <rpcndr.h>
#endif /* __RPCNDR_H_VERSION__ */

#ifndef COM_NO_WINDOWS_H
#include <windows.h>
#include <ole2.h>
#endif /*COM_NO_WINDOWS_H*/
#ifndef __comio_h__
#define __comio_h__
#ifndef __comio_p_h__
#define __comio_p_h__


#pragma once

// Ensure that the setting of the /ns_prefix command line switch is consistent for all headers.
// If you get an error from the compiler indicating "warning C4005: 'CHECK_NS_PREFIX_STATE': macro redefinition", this
// indicates that you have included two different headers with different settings for the /ns_prefix MIDL command line switch
#if !defined(DISABLE_NS_PREFIX_CHECKS)
#define CHECK_NS_PREFIX_STATE "always"
#endif // !defined(DISABLE_NS_PREFIX_CHECKS)


#pragma push_macro("MIDL_CONST_ID")
#undef MIDL_CONST_ID
#define MIDL_CONST_ID const __declspec(selectany)


//  API Contract Inclusion Definitions
#if !defined(SPECIFIC_API_CONTRACT_DEFINITIONS)
#if !defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_ACTIVATEDEVENTSCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_ACTIVATION_ACTIVATEDEVENTSCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_ACTIVATEDEVENTSCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_ACTIVATIONCAMERASETTINGSCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_ACTIVATION_ACTIVATIONCAMERASETTINGSCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_ACTIVATIONCAMERASETTINGSCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_CONTACTACTIVATEDEVENTSCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_ACTIVATION_CONTACTACTIVATEDEVENTSCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_CONTACTACTIVATEDEVENTSCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_WEBUISEARCHACTIVATEDEVENTSCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_ACTIVATION_WEBUISEARCHACTIVATEDEVENTSCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_APPLICATIONMODEL_ACTIVATION_WEBUISEARCHACTIVATEDEVENTSCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_BACKGROUND_BACKGROUNDALARMAPPLICATIONCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_BACKGROUND_BACKGROUNDALARMAPPLICATIONCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_APPLICATIONMODEL_BACKGROUND_BACKGROUNDALARMAPPLICATIONCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_CALLS_BACKGROUND_CALLSBACKGROUNDCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_CALLS_BACKGROUND_CALLSBACKGROUNDCONTRACT_VERSION 0x40000
#endif // defined(WINDOWS_APPLICATIONMODEL_CALLS_BACKGROUND_CALLSBACKGROUNDCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_CALLS_CALLSPHONECONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_CALLS_CALLSPHONECONTRACT_VERSION 0x70000
#endif // defined(WINDOWS_APPLICATIONMODEL_CALLS_CALLSPHONECONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_CALLS_CALLSVOIPCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_CALLS_CALLSVOIPCONTRACT_VERSION 0x50000
#endif // defined(WINDOWS_APPLICATIONMODEL_CALLS_CALLSVOIPCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_CALLS_LOCKSCREENCALLCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_CALLS_LOCKSCREENCALLCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_APPLICATIONMODEL_CALLS_LOCKSCREENCALLCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_COMMUNICATIONBLOCKING_COMMUNICATIONBLOCKINGCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_COMMUNICATIONBLOCKING_COMMUNICATIONBLOCKINGCONTRACT_VERSION 0x20000
#endif // defined(WINDOWS_APPLICATIONMODEL_COMMUNICATIONBLOCKING_COMMUNICATIONBLOCKINGCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_FULLTRUSTAPPCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_FULLTRUSTAPPCONTRACT_VERSION 0x20000
#endif // defined(WINDOWS_APPLICATIONMODEL_FULLTRUSTAPPCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_SEARCH_SEARCHCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_SEARCH_SEARCHCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_APPLICATIONMODEL_SEARCH_SEARCHCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_STARTUPTASKCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_STARTUPTASKCONTRACT_VERSION 0x30000
#endif // defined(WINDOWS_APPLICATIONMODEL_STARTUPTASKCONTRACT_VERSION)

#if !defined(WINDOWS_APPLICATIONMODEL_WALLET_WALLETCONTRACT_VERSION)
#define WINDOWS_APPLICATIONMODEL_WALLET_WALLETCONTRACT_VERSION 0x20000
#endif // defined(WINDOWS_APPLICATIONMODEL_WALLET_WALLETCONTRACT_VERSION)

#if !defined(WINDOWS_DEVICES_POWER_POWERGRIDAPICONTRACT_VERSION)
#define WINDOWS_DEVICES_POWER_POWERGRIDAPICONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_DEVICES_POWER_POWERGRIDAPICONTRACT_VERSION)

#if !defined(WINDOWS_DEVICES_PRINTERS_EXTENSIONS_EXTENSIONSCONTRACT_VERSION)
#define WINDOWS_DEVICES_PRINTERS_EXTENSIONS_EXTENSIONSCONTRACT_VERSION 0x20000
#endif // defined(WINDOWS_DEVICES_PRINTERS_EXTENSIONS_EXTENSIONSCONTRACT_VERSION)

#if !defined(WINDOWS_DEVICES_SMARTCARDS_SMARTCARDBACKGROUNDTRIGGERCONTRACT_VERSION)
#define WINDOWS_DEVICES_SMARTCARDS_SMARTCARDBACKGROUNDTRIGGERCONTRACT_VERSION 0x30000
#endif // defined(WINDOWS_DEVICES_SMARTCARDS_SMARTCARDBACKGROUNDTRIGGERCONTRACT_VERSION)

#if !defined(WINDOWS_DEVICES_SMARTCARDS_SMARTCARDEMULATORCONTRACT_VERSION)
#define WINDOWS_DEVICES_SMARTCARDS_SMARTCARDEMULATORCONTRACT_VERSION 0x60000
#endif // defined(WINDOWS_DEVICES_SMARTCARDS_SMARTCARDEMULATORCONTRACT_VERSION)

#if !defined(WINDOWS_DEVICES_SMS_LEGACYSMSAPICONTRACT_VERSION)
#define WINDOWS_DEVICES_SMS_LEGACYSMSAPICONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_DEVICES_SMS_LEGACYSMSAPICONTRACT_VERSION)

#if !defined(WINDOWS_FOUNDATION_FOUNDATIONCONTRACT_VERSION)
#define WINDOWS_FOUNDATION_FOUNDATIONCONTRACT_VERSION 0x40000
#endif // defined(WINDOWS_FOUNDATION_FOUNDATIONCONTRACT_VERSION)

#if !defined(WINDOWS_FOUNDATION_UNIVERSALAPICONTRACT_VERSION)
#define WINDOWS_FOUNDATION_UNIVERSALAPICONTRACT_VERSION 0x130000
#endif // defined(WINDOWS_FOUNDATION_UNIVERSALAPICONTRACT_VERSION)

#if !defined(WINDOWS_GAMING_INPUT_GAMINGINPUTPREVIEWCONTRACT_VERSION)
#define WINDOWS_GAMING_INPUT_GAMINGINPUTPREVIEWCONTRACT_VERSION 0x20000
#endif // defined(WINDOWS_GAMING_INPUT_GAMINGINPUTPREVIEWCONTRACT_VERSION)

#if !defined(WINDOWS_GLOBALIZATION_GLOBALIZATIONJAPANESEPHONETICANALYZERCONTRACT_VERSION)
#define WINDOWS_GLOBALIZATION_GLOBALIZATIONJAPANESEPHONETICANALYZERCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_GLOBALIZATION_GLOBALIZATIONJAPANESEPHONETICANALYZERCONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_CAPTURE_APPBROADCASTCONTRACT_VERSION)
#define WINDOWS_MEDIA_CAPTURE_APPBROADCASTCONTRACT_VERSION 0x20000
#endif // defined(WINDOWS_MEDIA_CAPTURE_APPBROADCASTCONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_CAPTURE_APPCAPTURECONTRACT_VERSION)
#define WINDOWS_MEDIA_CAPTURE_APPCAPTURECONTRACT_VERSION 0x40000
#endif // defined(WINDOWS_MEDIA_CAPTURE_APPCAPTURECONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_CAPTURE_APPCAPTUREMETADATACONTRACT_VERSION)
#define WINDOWS_MEDIA_CAPTURE_APPCAPTUREMETADATACONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_MEDIA_CAPTURE_APPCAPTUREMETADATACONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_CAPTURE_CAMERACAPTUREUICONTRACT_VERSION)
#define WINDOWS_MEDIA_CAPTURE_CAMERACAPTUREUICONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_MEDIA_CAPTURE_CAMERACAPTUREUICONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_CAPTURE_GAMEBARCONTRACT_VERSION)
#define WINDOWS_MEDIA_CAPTURE_GAMEBARCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_MEDIA_CAPTURE_GAMEBARCONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_DEVICES_CALLCONTROLCONTRACT_VERSION)
#define WINDOWS_MEDIA_DEVICES_CALLCONTROLCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_MEDIA_DEVICES_CALLCONTROLCONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_MEDIACONTROLCONTRACT_VERSION)
#define WINDOWS_MEDIA_MEDIACONTROLCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_MEDIA_MEDIACONTROLCONTRACT_VERSION)

#if !defined(WINDOWS_MEDIA_PROTECTION_PROTECTIONRENEWALCONTRACT_VERSION)
#define WINDOWS_MEDIA_PROTECTION_PROTECTIONRENEWALCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_MEDIA_PROTECTION_PROTECTIONRENEWALCONTRACT_VERSION)

#if !defined(WINDOWS_NETWORKING_CONNECTIVITY_WWANCONTRACT_VERSION)
#define WINDOWS_NETWORKING_CONNECTIVITY_WWANCONTRACT_VERSION 0x30000
#endif // defined(WINDOWS_NETWORKING_CONNECTIVITY_WWANCONTRACT_VERSION)

#if !defined(WINDOWS_NETWORKING_SOCKETS_CONTROLCHANNELTRIGGERCONTRACT_VERSION)
#define WINDOWS_NETWORKING_SOCKETS_CONTROLCHANNELTRIGGERCONTRACT_VERSION 0x30000
#endif // defined(WINDOWS_NETWORKING_SOCKETS_CONTROLCHANNELTRIGGERCONTRACT_VERSION)

#if !defined(WINDOWS_PHONE_PHONECONTRACT_VERSION)
#define WINDOWS_PHONE_PHONECONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_PHONE_PHONECONTRACT_VERSION)

#if !defined(WINDOWS_PHONE_PHONEINTERNALCONTRACT_VERSION)
#define WINDOWS_PHONE_PHONEINTERNALCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_PHONE_PHONEINTERNALCONTRACT_VERSION)

#if !defined(WINDOWS_SECURITY_ENTERPRISEDATA_ENTERPRISEDATACONTRACT_VERSION)
#define WINDOWS_SECURITY_ENTERPRISEDATA_ENTERPRISEDATACONTRACT_VERSION 0x50000
#endif // defined(WINDOWS_SECURITY_ENTERPRISEDATA_ENTERPRISEDATACONTRACT_VERSION)

#if !defined(WINDOWS_STORAGE_PROVIDER_CLOUDFILESCONTRACT_VERSION)
#define WINDOWS_STORAGE_PROVIDER_CLOUDFILESCONTRACT_VERSION 0x70000
#endif // defined(WINDOWS_STORAGE_PROVIDER_CLOUDFILESCONTRACT_VERSION)

#if !defined(WINDOWS_SYSTEM_SYSTEMMANAGEMENTCONTRACT_VERSION)
#define WINDOWS_SYSTEM_SYSTEMMANAGEMENTCONTRACT_VERSION 0x70000
#endif // defined(WINDOWS_SYSTEM_SYSTEMMANAGEMENTCONTRACT_VERSION)

#if !defined(WINDOWS_UI_CORE_COREWINDOWDIALOGSCONTRACT_VERSION)
#define WINDOWS_UI_CORE_COREWINDOWDIALOGSCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_UI_CORE_COREWINDOWDIALOGSCONTRACT_VERSION)

#if !defined(WINDOWS_UI_VIEWMANAGEMENT_VIEWMANAGEMENTVIEWSCALINGCONTRACT_VERSION)
#define WINDOWS_UI_VIEWMANAGEMENT_VIEWMANAGEMENTVIEWSCALINGCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_UI_VIEWMANAGEMENT_VIEWMANAGEMENTVIEWSCALINGCONTRACT_VERSION)

#if !defined(WINDOWS_UI_WEBUI_CORE_WEBUICOMMANDBARCONTRACT_VERSION)
#define WINDOWS_UI_WEBUI_CORE_WEBUICOMMANDBARCONTRACT_VERSION 0x10000
#endif // defined(WINDOWS_UI_WEBUI_CORE_WEBUICOMMANDBARCONTRACT_VERSION)

#endif // defined(SPECIFIC_API_CONTRACT_DEFINITIONS)


// Header files for imported files
#include "inspectable.h"
#include "AsyncInfo.h"
#include "EventToken.h"
#include "Windows.Foundation.h"
#include "Windows.Storage.Streams.h"
// Importing Collections header
#include <windows.foundation.collections.h>

#if defined(__cplusplus) && !defined(CINTERFACE)
/* Forward Declarations */
#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface IConnectionListener;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener ABI::Pbk20191::WinAsync::Comio::IConnectionListener

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface IConnectionReceivedArgs;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs ABI::Pbk20191::WinAsync::Comio::IConnectionReceivedArgs

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface IHyperVBind;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind ABI::Pbk20191::WinAsync::Comio::IHyperVBind

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface IHyperVConnector;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector ABI::Pbk20191::WinAsync::Comio::IHyperVConnector

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface INamedPipeBind;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind ABI::Pbk20191::WinAsync::Comio::INamedPipeBind

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface INamedPipeConnector;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector ABI::Pbk20191::WinAsync::Comio::INamedPipeConnector

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface ITransport;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport ABI::Pbk20191::WinAsync::Comio::ITransport

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface ITransportStream;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream ABI::Pbk20191::WinAsync::Comio::ITransportStream

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface IUnixBind;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind ABI::Pbk20191::WinAsync::Comio::IUnixBind

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                interface IUnixConnector;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector ABI::Pbk20191::WinAsync::Comio::IUnixConnector

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__

// Parameterized interface forward declarations (C++)

// Collection interface definitions


#ifndef DEF___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_USE
#define DEF___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_USE
#if !defined(RO_NO_TEMPLATE_NAME)
namespace ABI { namespace Windows { namespace Foundation {
template <>
struct __declspec(uuid("a464a93a-1a22-50c6-b3f6-c488a5f5f893"))
IAsyncOperationCompletedHandler<ABI::Pbk20191::WinAsync::Comio::ITransportStream*> : IAsyncOperationCompletedHandler_impl<ABI::Pbk20191::WinAsync::Comio::ITransportStream*> 
{
    static const wchar_t* z_get_rc_name_impl() 
    {
        return L"Windows.Foundation.AsyncOperationCompletedHandler`1<Pbk20191.WinAsync.Comio.ITransportStream>"; 
    }
};
// Define a typedef for the parameterized interface specialization's mangled name.
// This allows code which uses the mangled name for the parameterized interface to access the
// correct parameterized interface specialization.
typedef IAsyncOperationCompletedHandler<ABI::Pbk20191::WinAsync::Comio::ITransportStream*> __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_t;
#define __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream ABI::Windows::Foundation::__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_t
/* Foundation */ } /* Windows */ } /* ABI */ } 

////  Define an alias for the C version of the interface for compatibility purposes.
//#define __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream ABI::Windows::Foundation::IAsyncOperationCompletedHandler<ABI::Pbk20191::WinAsync::Comio::ITransportStream*>
//#define __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_t ABI::Windows::Foundation::IAsyncOperationCompletedHandler<ABI::Pbk20191::WinAsync::Comio::ITransportStream*>
#endif // !defined(RO_NO_TEMPLATE_NAME)
#endif /* DEF___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_USE */





#ifndef DEF___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_USE
#define DEF___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_USE
#if !defined(RO_NO_TEMPLATE_NAME)
namespace ABI { namespace Windows { namespace Foundation {
template <>
struct __declspec(uuid("f68136b7-f950-5f98-9399-310a0df73ad6"))
IAsyncOperation<ABI::Pbk20191::WinAsync::Comio::ITransportStream*> : IAsyncOperation_impl<ABI::Pbk20191::WinAsync::Comio::ITransportStream*> 
{
    static const wchar_t* z_get_rc_name_impl() 
    {
        return L"Windows.Foundation.IAsyncOperation`1<Pbk20191.WinAsync.Comio.ITransportStream>"; 
    }
};
// Define a typedef for the parameterized interface specialization's mangled name.
// This allows code which uses the mangled name for the parameterized interface to access the
// correct parameterized interface specialization.
typedef IAsyncOperation<ABI::Pbk20191::WinAsync::Comio::ITransportStream*> __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_t;
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream ABI::Windows::Foundation::__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_t
/* Foundation */ } /* Windows */ } /* ABI */ } 

////  Define an alias for the C version of the interface for compatibility purposes.
//#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream ABI::Windows::Foundation::IAsyncOperation<ABI::Pbk20191::WinAsync::Comio::ITransportStream*>
//#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_t ABI::Windows::Foundation::IAsyncOperation<ABI::Pbk20191::WinAsync::Comio::ITransportStream*>
#endif // !defined(RO_NO_TEMPLATE_NAME)
#endif /* DEF___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_USE */






#ifndef DEF___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_USE
#define DEF___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_USE
#if !defined(RO_NO_TEMPLATE_NAME)
namespace ABI { namespace Windows { namespace Foundation {
template <>
struct __declspec(uuid("0d47445c-a1be-507c-8ca9-d728a0fe89d8"))
ITypedEventHandler<ABI::Pbk20191::WinAsync::Comio::IConnectionListener*,ABI::Pbk20191::WinAsync::Comio::IConnectionReceivedArgs*> : ITypedEventHandler_impl<ABI::Pbk20191::WinAsync::Comio::IConnectionListener*,ABI::Pbk20191::WinAsync::Comio::IConnectionReceivedArgs*> 
{
    static const wchar_t* z_get_rc_name_impl() 
    {
        return L"Windows.Foundation.TypedEventHandler`2<Pbk20191.WinAsync.Comio.IConnectionListener, Pbk20191.WinAsync.Comio.IConnectionReceivedArgs>"; 
    }
};
// Define a typedef for the parameterized interface specialization's mangled name.
// This allows code which uses the mangled name for the parameterized interface to access the
// correct parameterized interface specialization.
typedef ITypedEventHandler<ABI::Pbk20191::WinAsync::Comio::IConnectionListener*,ABI::Pbk20191::WinAsync::Comio::IConnectionReceivedArgs*> __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_t;
#define __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs ABI::Windows::Foundation::__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_t
/* Foundation */ } /* Windows */ } /* ABI */ } 

////  Define an alias for the C version of the interface for compatibility purposes.
//#define __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs ABI::Windows::Foundation::ITypedEventHandler<ABI::Pbk20191::WinAsync::Comio::IConnectionListener*,ABI::Pbk20191::WinAsync::Comio::IConnectionReceivedArgs*>
//#define __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_t ABI::Windows::Foundation::ITypedEventHandler<ABI::Pbk20191::WinAsync::Comio::IConnectionListener*,ABI::Pbk20191::WinAsync::Comio::IConnectionReceivedArgs*>
#endif // !defined(RO_NO_TEMPLATE_NAME)
#endif /* DEF___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_USE */





#ifndef ____x_ABI_CWindows_CFoundation_CIAsyncAction_FWD_DEFINED__
#define ____x_ABI_CWindows_CFoundation_CIAsyncAction_FWD_DEFINED__
namespace ABI {
    namespace Windows {
        namespace Foundation {
            interface IAsyncAction;
        } /* Foundation */
    } /* Windows */
} /* ABI */
#define __x_ABI_CWindows_CFoundation_CIAsyncAction ABI::Windows::Foundation::IAsyncAction

#endif // ____x_ABI_CWindows_CFoundation_CIAsyncAction_FWD_DEFINED__


#ifndef ____x_ABI_CWindows_CFoundation_CIClosable_FWD_DEFINED__
#define ____x_ABI_CWindows_CFoundation_CIClosable_FWD_DEFINED__
namespace ABI {
    namespace Windows {
        namespace Foundation {
            interface IClosable;
        } /* Foundation */
    } /* Windows */
} /* ABI */
#define __x_ABI_CWindows_CFoundation_CIClosable ABI::Windows::Foundation::IClosable

#endif // ____x_ABI_CWindows_CFoundation_CIClosable_FWD_DEFINED__




#ifndef ____x_ABI_CWindows_CStorage_CStreams_CIInputStream_FWD_DEFINED__
#define ____x_ABI_CWindows_CStorage_CStreams_CIInputStream_FWD_DEFINED__
namespace ABI {
    namespace Windows {
        namespace Storage {
            namespace Streams {
                interface IInputStream;
            } /* Streams */
        } /* Storage */
    } /* Windows */
} /* ABI */
#define __x_ABI_CWindows_CStorage_CStreams_CIInputStream ABI::Windows::Storage::Streams::IInputStream

#endif // ____x_ABI_CWindows_CStorage_CStreams_CIInputStream_FWD_DEFINED__


#ifndef ____x_ABI_CWindows_CStorage_CStreams_CIOutputStream_FWD_DEFINED__
#define ____x_ABI_CWindows_CStorage_CStreams_CIOutputStream_FWD_DEFINED__
namespace ABI {
    namespace Windows {
        namespace Storage {
            namespace Streams {
                interface IOutputStream;
            } /* Streams */
        } /* Storage */
    } /* Windows */
} /* ABI */
#define __x_ABI_CWindows_CStorage_CStreams_CIOutputStream ABI::Windows::Storage::Streams::IOutputStream

#endif // ____x_ABI_CWindows_CStorage_CStreams_CIOutputStream_FWD_DEFINED__















namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class ConnectionReceivedArgs;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */


namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class HyperVConnectionListener;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */


namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class HyperVConnector;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */


namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class NamedPipeConnectionListener;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */


namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class NamedPipeConnector;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */


namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class TransportStream;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */


namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class UnixConnectionListener;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */


namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                class UnixConnector;
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */










/*
 *
 * Interface Pbk20191.WinAsync.Comio.IConnectionListener
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IConnectionListener[] = L"Pbk20191.WinAsync.Comio.IConnectionListener";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("D0E8FF19-BA8B-5263-9551-0F052DADA51B")] */
                MIDL_INTERFACE("D0E8FF19-BA8B-5263-9551-0F052DADA51B")
                IConnectionListener : public IInspectable
                {
                public:
                    /* [eventadd] */virtual HRESULT STDMETHODCALLTYPE add_ConnectionReceived(
                        /* [in] */__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * handler,
                        /* [retval, out] */EventRegistrationToken * __returnValue_add_ConnectionReceived__
                        ) = 0;
                    /* [eventremove] */virtual HRESULT STDMETHODCALLTYPE remove_ConnectionReceived(
                        /* [in] */EventRegistrationToken token
                        ) = 0;
                    /* [propget] */virtual HRESULT STDMETHODCALLTYPE get_Information(
                        /* [retval, out] */HSTRING * __returnValue_get_Information__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_IConnectionListener=__uuidof(IConnectionListener);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IConnectionReceivedArgs
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IConnectionReceivedArgs[] = L"Pbk20191.WinAsync.Comio.IConnectionReceivedArgs";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("661712AC-63E3-5243-A91A-6409D33FC02B")] */
                MIDL_INTERFACE("661712AC-63E3-5243-A91A-6409D33FC02B")
                IConnectionReceivedArgs : public IInspectable
                {
                public:
                    /* [propget] */virtual HRESULT STDMETHODCALLTYPE get_Stream(
                        /* [retval, out] */ABI::Pbk20191::WinAsync::Comio::ITransportStream * * __returnValue_get_Stream__
                        ) = 0;
                    /* [propget] */virtual HRESULT STDMETHODCALLTYPE get_RemoteEndpoint(
                        /* [retval, out] */HSTRING * __returnValue_get_RemoteEndpoint__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_IConnectionReceivedArgs=__uuidof(IConnectionReceivedArgs);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IHyperVBind
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IHyperVBind[] = L"Pbk20191.WinAsync.Comio.IHyperVBind";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("38DEC2E0-5F8F-5B4B-8CEB-E57D53390C1C")] */
                MIDL_INTERFACE("38DEC2E0-5F8F-5B4B-8CEB-E57D53390C1C")
                IHyperVBind : public IInspectable
                {
                public:
                    virtual HRESULT STDMETHODCALLTYPE BindEndpointAsync(
                        /* [in] */HSTRING endpoint,
                        /* [retval, out] */ABI::Windows::Foundation::IAsyncAction * * __returnValue_BindEndpointAsync__
                        ) = 0;
                    virtual HRESULT STDMETHODCALLTYPE BindServiceNameAsync(
                        /* [in] */HSTRING serviceName,
                        /* [retval, out] */ABI::Windows::Foundation::IAsyncAction * * __returnValue_BindServiceNameAsync__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_IHyperVBind=__uuidof(IHyperVBind);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IHyperVConnector
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IHyperVConnector[] = L"Pbk20191.WinAsync.Comio.IHyperVConnector";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("7668644D-7C82-57DC-A371-B7404F3E99B0")] */
                MIDL_INTERFACE("7668644D-7C82-57DC-A371-B7404F3E99B0")
                IHyperVConnector : public IInspectable
                {
                public:
                    virtual HRESULT STDMETHODCALLTYPE ConnectAsync(
                        /* [in] */HSTRING endpoint,
                        /* [retval, out] */__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * * __returnValue_ConnectAsync__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_IHyperVConnector=__uuidof(IHyperVConnector);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.INamedPipeBind
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_INamedPipeBind[] = L"Pbk20191.WinAsync.Comio.INamedPipeBind";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("4E5DB519-5FB8-5E37-81C8-B08A4A10B4FA")] */
                MIDL_INTERFACE("4E5DB519-5FB8-5E37-81C8-B08A4A10B4FA")
                INamedPipeBind : public IInspectable
                {
                public:
                    virtual HRESULT STDMETHODCALLTYPE BindAsync(
                        /* [in] */HSTRING name,
                        /* [retval, out] */ABI::Windows::Foundation::IAsyncAction * * __returnValue_BindAsync__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_INamedPipeBind=__uuidof(INamedPipeBind);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.INamedPipeConnector
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_INamedPipeConnector[] = L"Pbk20191.WinAsync.Comio.INamedPipeConnector";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("9AD13FB2-9C7F-5286-A9CF-71F9B4C3E08F")] */
                MIDL_INTERFACE("9AD13FB2-9C7F-5286-A9CF-71F9B4C3E08F")
                INamedPipeConnector : public IInspectable
                {
                public:
                    virtual HRESULT STDMETHODCALLTYPE ConnectAsync(
                        /* [in] */HSTRING name,
                        /* [retval, out] */__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * * __returnValue_ConnectAsync__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_INamedPipeConnector=__uuidof(INamedPipeConnector);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.ITransport
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_ITransport[] = L"Pbk20191.WinAsync.Comio.ITransport";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("9455E7DE-0C4F-5D23-902A-A44EBF0A4AE6")] */
                MIDL_INTERFACE("9455E7DE-0C4F-5D23-902A-A44EBF0A4AE6")
                ITransport : public IInspectable
                {
                public:
                    /* [propget] */virtual HRESULT STDMETHODCALLTYPE get_Transport(
                        /* [retval, out] */HSTRING * __returnValue_get_Transport__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_ITransport=__uuidof(ITransport);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransport;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.ITransportStream
 *
 * Any object which implements this interface must also implement the following interfaces:
 *     Windows.Storage.Streams.IInputStream
 *     Windows.Storage.Streams.IOutputStream
 *     Windows.Foundation.IClosable
 *
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_ITransportStream[] = L"Pbk20191.WinAsync.Comio.ITransportStream";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("AD413F92-1757-501D-B24A-765E6B4E883C")] */
                MIDL_INTERFACE("AD413F92-1757-501D-B24A-765E6B4E883C")
                ITransportStream : public IInspectable
                {
                public:
                    
                };

                MIDL_CONST_ID IID & IID_ITransportStream=__uuidof(ITransportStream);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IUnixBind
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IUnixBind[] = L"Pbk20191.WinAsync.Comio.IUnixBind";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("70FACC3F-8F25-53F5-8BA3-9C919F1F9534")] */
                MIDL_INTERFACE("70FACC3F-8F25-53F5-8BA3-9C919F1F9534")
                IUnixBind : public IInspectable
                {
                public:
                    virtual HRESULT STDMETHODCALLTYPE BindAsync(
                        /* [in] */HSTRING path,
                        /* [retval, out] */ABI::Windows::Foundation::IAsyncAction * * __returnValue_BindAsync__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_IUnixBind=__uuidof(IUnixBind);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IUnixConnector
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IUnixConnector[] = L"Pbk20191.WinAsync.Comio.IUnixConnector";
namespace ABI {
    namespace Pbk20191 {
        namespace WinAsync {
            namespace Comio {
                /* [object, version, uuid("79876E3D-A244-5AA3-93FE-FF040F1522EC")] */
                MIDL_INTERFACE("79876E3D-A244-5AA3-93FE-FF040F1522EC")
                IUnixConnector : public IInspectable
                {
                public:
                    virtual HRESULT STDMETHODCALLTYPE ConnectAsync(
                        /* [in] */HSTRING path,
                        /* [retval, out] */__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * * __returnValue_ConnectAsync__
                        ) = 0;
                    
                };

                MIDL_CONST_ID IID & IID_IUnixConnector=__uuidof(IUnixConnector);
                
            } /* Comio */
        } /* WinAsync */
    } /* Pbk20191 */
} /* ABI */

EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__) */


/*
 *
 * Class Pbk20191.WinAsync.Comio.ConnectionReceivedArgs
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionReceivedArgs ** Default Interface **
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_ConnectionReceivedArgs_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_ConnectionReceivedArgs_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_ConnectionReceivedArgs[] = L"Pbk20191.WinAsync.Comio.ConnectionReceivedArgs";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.HyperVConnectionListener
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionListener ** Default Interface **
 *    Pbk20191.WinAsync.Comio.ITransport
 *    Pbk20191.WinAsync.Comio.IHyperVBind
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnectionListener_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnectionListener_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_HyperVConnectionListener[] = L"Pbk20191.WinAsync.Comio.HyperVConnectionListener";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.HyperVConnector
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransport ** Default Interface **
 *    Pbk20191.WinAsync.Comio.IHyperVConnector
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnector_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnector_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_HyperVConnector[] = L"Pbk20191.WinAsync.Comio.HyperVConnector";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.NamedPipeConnectionListener
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionListener ** Default Interface **
 *    Pbk20191.WinAsync.Comio.ITransport
 *    Pbk20191.WinAsync.Comio.INamedPipeBind
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_NamedPipeConnectionListener[] = L"Pbk20191.WinAsync.Comio.NamedPipeConnectionListener";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.NamedPipeConnector
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransport ** Default Interface **
 *    Pbk20191.WinAsync.Comio.INamedPipeConnector
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnector_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnector_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_NamedPipeConnector[] = L"Pbk20191.WinAsync.Comio.NamedPipeConnector";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.TransportStream
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransportStream ** Default Interface **
 *    Windows.Storage.Streams.IInputStream
 *    Windows.Storage.Streams.IOutputStream
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_TransportStream_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_TransportStream_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_TransportStream[] = L"Pbk20191.WinAsync.Comio.TransportStream";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.UnixConnectionListener
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionListener ** Default Interface **
 *    Pbk20191.WinAsync.Comio.ITransport
 *    Pbk20191.WinAsync.Comio.IUnixBind
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnectionListener_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnectionListener_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_UnixConnectionListener[] = L"Pbk20191.WinAsync.Comio.UnixConnectionListener";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.UnixConnector
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransport ** Default Interface **
 *    Pbk20191.WinAsync.Comio.IUnixConnector
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnector_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnector_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_UnixConnector[] = L"Pbk20191.WinAsync.Comio.UnixConnector";
#endif




#else // !defined(__cplusplus)
/* Forward Declarations */
#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransport __x_ABI_CPbk20191_CWinAsync_CComio_CITransport;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector;

#endif // ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__

// Parameterized interface forward declarations (C)

// Collection interface definitions

#if !defined(____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__)
#define ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__

typedef interface __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream;

//  Declare the parameterized interface IID.
EXTERN_C const IID IID___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream;

// Forward declare the async operation.
typedef interface __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream;

typedef struct __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl
{
    BEGIN_INTERFACE

    HRESULT ( STDMETHODCALLTYPE *QueryInterface )(__RPC__in __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
        /* [in] */ __RPC__in REFIID riid,
        /* [annotation][iid_is][out] */ 
        _COM_Outptr_  void **ppvObject);
    ULONG ( STDMETHODCALLTYPE *AddRef )(__RPC__in __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);
    ULONG ( STDMETHODCALLTYPE *Release )(__RPC__in __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);

    HRESULT ( STDMETHODCALLTYPE *Invoke )(__RPC__in __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,/* [in] */ __RPC__in_opt __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream *asyncInfo, /* [in] */ AsyncStatus status);
    END_INTERFACE
} __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl;

interface __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream
{
    CONST_VTBL struct __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 
#define __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 
#define __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 

#define __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_Invoke(This,asyncInfo,status)	\
    ( (This)->lpVtbl -> Invoke(This,asyncInfo,status) ) 
#endif /* COBJMACROS */


#endif // ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__



#if !defined(____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__)
#define ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__

typedef interface __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream;

//  Declare the parameterized interface IID.
EXTERN_C const IID IID___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream;

typedef struct __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
        /* [in] */ __RPC__in REFIID riid,
        /* [annotation][iid_is][out] */ 
        _COM_Outptr_  void **ppvObject);
    ULONG ( STDMETHODCALLTYPE *AddRef )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);
    ULONG ( STDMETHODCALLTYPE *Release )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);

    HRESULT ( STDMETHODCALLTYPE *GetIids )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
        /* [out] */ __RPC__out ULONG *iidCount,
        /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids);
    HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This, /* [out] */ __RPC__deref_out_opt HSTRING *className);
    HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This, /* [out] */ __RPC__out TrustLevel *trustLevel);

    /* [propput] */ HRESULT ( STDMETHODCALLTYPE *put_Completed )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This, /* [in] */ __RPC__in_opt __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream *handler);
    /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_Completed )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This, /* [retval][out] */ __RPC__deref_out_opt __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream **handler);
    HRESULT ( STDMETHODCALLTYPE *GetResults )(__RPC__in __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This, /* [retval][out] */ __RPC__out __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * *results);
    END_INTERFACE
} __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl;

interface __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream
{
    CONST_VTBL struct __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 

#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 

#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_put_Completed(This,handler)	\
    ( (This)->lpVtbl -> put_Completed(This,handler) ) 
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_get_Completed(This,handler)	\
    ( (This)->lpVtbl -> get_Completed(This,handler) ) 
#define __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_GetResults(This,results)	\
    ( (This)->lpVtbl -> GetResults(This,results) ) 
#endif /* COBJMACROS */


#endif // ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__




#if !defined(____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_INTERFACE_DEFINED__)
#define ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_INTERFACE_DEFINED__

typedef interface __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs;

//  Declare the parameterized interface IID.
EXTERN_C const IID IID___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs;

typedef struct __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface )(__RPC__in __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This,
        /* [in] */ __RPC__in REFIID riid,
        /* [annotation][iid_is][out] */ 
        _COM_Outptr_  void **ppvObject);
    ULONG ( STDMETHODCALLTYPE *AddRef )(__RPC__in __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This);
    ULONG ( STDMETHODCALLTYPE *Release )(__RPC__in __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This);

    HRESULT ( STDMETHODCALLTYPE *Invoke )(__RPC__in __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This,/* [in] */ __RPC__in_opt __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * sender,/* [in] */ __RPC__in_opt __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * e);
    END_INTERFACE
} __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsVtbl;

interface __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs
{
    CONST_VTBL struct __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_Invoke(This,sender,e)	\
    ( (This)->lpVtbl -> Invoke(This,sender,e) ) 
#endif /* COBJMACROS */



#endif // ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_INTERFACE_DEFINED__



#ifndef ____x_ABI_CWindows_CFoundation_CIAsyncAction_FWD_DEFINED__
#define ____x_ABI_CWindows_CFoundation_CIAsyncAction_FWD_DEFINED__
typedef interface __x_ABI_CWindows_CFoundation_CIAsyncAction __x_ABI_CWindows_CFoundation_CIAsyncAction;

#endif // ____x_ABI_CWindows_CFoundation_CIAsyncAction_FWD_DEFINED__


#ifndef ____x_ABI_CWindows_CFoundation_CIClosable_FWD_DEFINED__
#define ____x_ABI_CWindows_CFoundation_CIClosable_FWD_DEFINED__
typedef interface __x_ABI_CWindows_CFoundation_CIClosable __x_ABI_CWindows_CFoundation_CIClosable;

#endif // ____x_ABI_CWindows_CFoundation_CIClosable_FWD_DEFINED__




#ifndef ____x_ABI_CWindows_CStorage_CStreams_CIInputStream_FWD_DEFINED__
#define ____x_ABI_CWindows_CStorage_CStreams_CIInputStream_FWD_DEFINED__
typedef interface __x_ABI_CWindows_CStorage_CStreams_CIInputStream __x_ABI_CWindows_CStorage_CStreams_CIInputStream;

#endif // ____x_ABI_CWindows_CStorage_CStreams_CIInputStream_FWD_DEFINED__


#ifndef ____x_ABI_CWindows_CStorage_CStreams_CIOutputStream_FWD_DEFINED__
#define ____x_ABI_CWindows_CStorage_CStreams_CIOutputStream_FWD_DEFINED__
typedef interface __x_ABI_CWindows_CStorage_CStreams_CIOutputStream __x_ABI_CWindows_CStorage_CStreams_CIOutputStream;

#endif // ____x_ABI_CWindows_CStorage_CStreams_CIOutputStream_FWD_DEFINED__































/*
 *
 * Interface Pbk20191.WinAsync.Comio.IConnectionListener
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IConnectionListener[] = L"Pbk20191.WinAsync.Comio.IConnectionListener";
/* [object, version, uuid("D0E8FF19-BA8B-5263-9551-0F052DADA51B")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
/* [eventadd] */HRESULT ( STDMETHODCALLTYPE *add_ConnectionReceived )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
        /* [in] */__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * handler,
        /* [retval, out] */EventRegistrationToken * __returnValue_add_ConnectionReceived__
        );
    /* [eventremove] */HRESULT ( STDMETHODCALLTYPE *remove_ConnectionReceived )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
        /* [in] */EventRegistrationToken token
        );
    /* [propget] */HRESULT ( STDMETHODCALLTYPE *get_Information )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
        /* [retval, out] */HSTRING * __returnValue_get_Information__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_add_ConnectionReceived(This,handler,__returnValue_add_ConnectionReceived__) \
    ( (This)->lpVtbl->add_ConnectionReceived(This,handler,__returnValue_add_ConnectionReceived__) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_remove_ConnectionReceived(This,token) \
    ( (This)->lpVtbl->remove_ConnectionReceived(This,token) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_get_Information(This,__returnValue_get_Information__) \
    ( (This)->lpVtbl->get_Information(This,__returnValue_get_Information__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IConnectionReceivedArgs
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IConnectionReceivedArgs[] = L"Pbk20191.WinAsync.Comio.IConnectionReceivedArgs";
/* [object, version, uuid("661712AC-63E3-5243-A91A-6409D33FC02B")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
/* [propget] */HRESULT ( STDMETHODCALLTYPE *get_Stream )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
        /* [retval, out] */__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * * __returnValue_get_Stream__
        );
    /* [propget] */HRESULT ( STDMETHODCALLTYPE *get_RemoteEndpoint )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
        /* [retval, out] */HSTRING * __returnValue_get_RemoteEndpoint__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_get_Stream(This,__returnValue_get_Stream__) \
    ( (This)->lpVtbl->get_Stream(This,__returnValue_get_Stream__) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_get_RemoteEndpoint(This,__returnValue_get_RemoteEndpoint__) \
    ( (This)->lpVtbl->get_RemoteEndpoint(This,__returnValue_get_RemoteEndpoint__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IHyperVBind
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IHyperVBind[] = L"Pbk20191.WinAsync.Comio.IHyperVBind";
/* [object, version, uuid("38DEC2E0-5F8F-5B4B-8CEB-E57D53390C1C")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
HRESULT ( STDMETHODCALLTYPE *BindEndpointAsync )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
        /* [in] */HSTRING endpoint,
        /* [retval, out] */__x_ABI_CWindows_CFoundation_CIAsyncAction * * __returnValue_BindEndpointAsync__
        );
    HRESULT ( STDMETHODCALLTYPE *BindServiceNameAsync )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
        /* [in] */HSTRING serviceName,
        /* [retval, out] */__x_ABI_CWindows_CFoundation_CIAsyncAction * * __returnValue_BindServiceNameAsync__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_BindEndpointAsync(This,endpoint,__returnValue_BindEndpointAsync__) \
    ( (This)->lpVtbl->BindEndpointAsync(This,endpoint,__returnValue_BindEndpointAsync__) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_BindServiceNameAsync(This,serviceName,__returnValue_BindServiceNameAsync__) \
    ( (This)->lpVtbl->BindServiceNameAsync(This,serviceName,__returnValue_BindServiceNameAsync__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IHyperVConnector
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IHyperVConnector[] = L"Pbk20191.WinAsync.Comio.IHyperVConnector";
/* [object, version, uuid("7668644D-7C82-57DC-A371-B7404F3E99B0")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
HRESULT ( STDMETHODCALLTYPE *ConnectAsync )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
        /* [in] */HSTRING endpoint,
        /* [retval, out] */__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * * __returnValue_ConnectAsync__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ConnectAsync(This,endpoint,__returnValue_ConnectAsync__) \
    ( (This)->lpVtbl->ConnectAsync(This,endpoint,__returnValue_ConnectAsync__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.INamedPipeBind
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_INamedPipeBind[] = L"Pbk20191.WinAsync.Comio.INamedPipeBind";
/* [object, version, uuid("4E5DB519-5FB8-5E37-81C8-B08A4A10B4FA")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
HRESULT ( STDMETHODCALLTYPE *BindAsync )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
        /* [in] */HSTRING name,
        /* [retval, out] */__x_ABI_CWindows_CFoundation_CIAsyncAction * * __returnValue_BindAsync__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_BindAsync(This,name,__returnValue_BindAsync__) \
    ( (This)->lpVtbl->BindAsync(This,name,__returnValue_BindAsync__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.INamedPipeConnector
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_INamedPipeConnector[] = L"Pbk20191.WinAsync.Comio.INamedPipeConnector";
/* [object, version, uuid("9AD13FB2-9C7F-5286-A9CF-71F9B4C3E08F")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
HRESULT ( STDMETHODCALLTYPE *ConnectAsync )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
        /* [in] */HSTRING name,
        /* [retval, out] */__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * * __returnValue_ConnectAsync__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ConnectAsync(This,name,__returnValue_ConnectAsync__) \
    ( (This)->lpVtbl->ConnectAsync(This,name,__returnValue_ConnectAsync__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.ITransport
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_ITransport[] = L"Pbk20191.WinAsync.Comio.ITransport";
/* [object, version, uuid("9455E7DE-0C4F-5D23-902A-A44EBF0A4AE6")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
/* [propget] */HRESULT ( STDMETHODCALLTYPE *get_Transport )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
        /* [retval, out] */HSTRING * __returnValue_get_Transport__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CITransportVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransport
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_get_Transport(This,__returnValue_get_Transport__) \
    ( (This)->lpVtbl->get_Transport(This,__returnValue_get_Transport__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransport;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.ITransportStream
 *
 * Any object which implements this interface must also implement the following interfaces:
 *     Windows.Storage.Streams.IInputStream
 *     Windows.Storage.Streams.IOutputStream
 *     Windows.Foundation.IClosable
 *
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_ITransportStream[] = L"Pbk20191.WinAsync.Comio.ITransportStream";
/* [object, version, uuid("AD413F92-1757-501D-B24A-765E6B4E883C")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IUnixBind
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IUnixBind[] = L"Pbk20191.WinAsync.Comio.IUnixBind";
/* [object, version, uuid("70FACC3F-8F25-53F5-8BA3-9C919F1F9534")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
HRESULT ( STDMETHODCALLTYPE *BindAsync )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
        /* [in] */HSTRING path,
        /* [retval, out] */__x_ABI_CWindows_CFoundation_CIAsyncAction * * __returnValue_BindAsync__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_BindAsync(This,path,__returnValue_BindAsync__) \
    ( (This)->lpVtbl->BindAsync(This,path,__returnValue_BindAsync__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__) */


/*
 *
 * Interface Pbk20191.WinAsync.Comio.IUnixConnector
 *
 */
#if !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__)
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__
extern const __declspec(selectany) _Null_terminated_ WCHAR InterfaceName_Pbk20191_WinAsync_Comio_IUnixConnector[] = L"Pbk20191.WinAsync.Comio.IUnixConnector";
/* [object, version, uuid("79876E3D-A244-5AA3-93FE-FF040F1522EC")] */
typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorVtbl
{
    BEGIN_INTERFACE
    HRESULT ( STDMETHODCALLTYPE *QueryInterface)(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
    /* [in] */ __RPC__in REFIID riid,
    /* [annotation][iid_is][out] */
    _COM_Outptr_  void **ppvObject
    );

ULONG ( STDMETHODCALLTYPE *AddRef )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This
    );

ULONG ( STDMETHODCALLTYPE *Release )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This
    );

HRESULT ( STDMETHODCALLTYPE *GetIids )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
    /* [out] */ __RPC__out ULONG *iidCount,
    /* [size_is][size_is][out] */ __RPC__deref_out_ecount_full_opt(*iidCount) IID **iids
    );

HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
    /* [out] */ __RPC__deref_out_opt HSTRING *className
    );

HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )(
    __RPC__in __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
    /* [OUT ] */ __RPC__out TrustLevel *trustLevel
    );
HRESULT ( STDMETHODCALLTYPE *ConnectAsync )(
        __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
        /* [in] */HSTRING path,
        /* [retval, out] */__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * * __returnValue_ConnectAsync__
        );
    END_INTERFACE
    
} __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorVtbl;

interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector
{
    CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorVtbl *lpVtbl;
};

#ifdef COBJMACROS
#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_QueryInterface(This,riid,ppvObject) \
( (This)->lpVtbl->QueryInterface(This,riid,ppvObject) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_AddRef(This) \
        ( (This)->lpVtbl->AddRef(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_Release(This) \
        ( (This)->lpVtbl->Release(This) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_GetIids(This,iidCount,iids) \
        ( (This)->lpVtbl->GetIids(This,iidCount,iids) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_GetRuntimeClassName(This,className) \
        ( (This)->lpVtbl->GetRuntimeClassName(This,className) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_GetTrustLevel(This,trustLevel) \
        ( (This)->lpVtbl->GetTrustLevel(This,trustLevel) )

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ConnectAsync(This,path,__returnValue_ConnectAsync__) \
    ( (This)->lpVtbl->ConnectAsync(This,path,__returnValue_ConnectAsync__) )


#endif /* COBJMACROS */


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector;
#endif /* !defined(____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__) */


/*
 *
 * Class Pbk20191.WinAsync.Comio.ConnectionReceivedArgs
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionReceivedArgs ** Default Interface **
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_ConnectionReceivedArgs_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_ConnectionReceivedArgs_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_ConnectionReceivedArgs[] = L"Pbk20191.WinAsync.Comio.ConnectionReceivedArgs";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.HyperVConnectionListener
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionListener ** Default Interface **
 *    Pbk20191.WinAsync.Comio.ITransport
 *    Pbk20191.WinAsync.Comio.IHyperVBind
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnectionListener_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnectionListener_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_HyperVConnectionListener[] = L"Pbk20191.WinAsync.Comio.HyperVConnectionListener";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.HyperVConnector
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransport ** Default Interface **
 *    Pbk20191.WinAsync.Comio.IHyperVConnector
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnector_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_HyperVConnector_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_HyperVConnector[] = L"Pbk20191.WinAsync.Comio.HyperVConnector";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.NamedPipeConnectionListener
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionListener ** Default Interface **
 *    Pbk20191.WinAsync.Comio.ITransport
 *    Pbk20191.WinAsync.Comio.INamedPipeBind
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnectionListener_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_NamedPipeConnectionListener[] = L"Pbk20191.WinAsync.Comio.NamedPipeConnectionListener";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.NamedPipeConnector
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransport ** Default Interface **
 *    Pbk20191.WinAsync.Comio.INamedPipeConnector
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnector_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_NamedPipeConnector_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_NamedPipeConnector[] = L"Pbk20191.WinAsync.Comio.NamedPipeConnector";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.TransportStream
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransportStream ** Default Interface **
 *    Windows.Storage.Streams.IInputStream
 *    Windows.Storage.Streams.IOutputStream
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_TransportStream_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_TransportStream_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_TransportStream[] = L"Pbk20191.WinAsync.Comio.TransportStream";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.UnixConnectionListener
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.IConnectionListener ** Default Interface **
 *    Pbk20191.WinAsync.Comio.ITransport
 *    Pbk20191.WinAsync.Comio.IUnixBind
 *    Windows.Foundation.IClosable
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnectionListener_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnectionListener_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_UnixConnectionListener[] = L"Pbk20191.WinAsync.Comio.UnixConnectionListener";
#endif


/*
 *
 * Class Pbk20191.WinAsync.Comio.UnixConnector
 *
 * RuntimeClass can be activated.
 *
 * Class implements the following interfaces:
 *    Pbk20191.WinAsync.Comio.ITransport ** Default Interface **
 *    Pbk20191.WinAsync.Comio.IUnixConnector
 *
 */

#ifndef RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnector_DEFINED
#define RUNTIMECLASS_Pbk20191_WinAsync_Comio_UnixConnector_DEFINED
extern const __declspec(selectany) _Null_terminated_ WCHAR RuntimeClass_Pbk20191_WinAsync_Comio_UnixConnector[] = L"Pbk20191.WinAsync.Comio.UnixConnector";
#endif




#endif // defined(__cplusplus)
#pragma pop_macro("MIDL_CONST_ID")
#endif // __comio_p_h__

#endif // __comio_h__
