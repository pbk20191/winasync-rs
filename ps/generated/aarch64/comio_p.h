

/* this ALWAYS GENERATED file contains the definitions for the interfaces */


 /* File created by MIDL compiler version 8.01.0628 */
/* at Tue Jan 19 12:14:07 2038
 */
/* Compiler settings for Pbk20191.WinAsync.Comio.idl:
    Oicf, W1, Zp8, env=Win64 (32b run), target_arch=ARM64 8.01.0628 
    protocol : all , ms_ext, c_ext, robust
    error checks: allocation ref bounds_check enum stub_data 
    VC __declspec() decoration level: 
         __declspec(uuid()), __declspec(selectany), __declspec(novtable)
         DECLSPEC_UUID(), MIDL_INTERFACE()
*/
/* @@MIDL_FILE_HEADING(  ) */



/* verify that the <rpcndr.h> version is high enough to compile this file*/
#ifndef __REQUIRED_RPCNDR_H_VERSION__
#define __REQUIRED_RPCNDR_H_VERSION__ 500
#endif

#include "rpc.h"
#include "rpcndr.h"

#ifndef __RPCNDR_H_VERSION__
#error this stub requires an updated version of <rpcndr.h>
#endif /* __RPCNDR_H_VERSION__ */

#ifndef COM_NO_WINDOWS_H
#include "windows.h"
#include "ole2.h"
#endif /*COM_NO_WINDOWS_H*/

#ifndef __comio_p_h__
#define __comio_p_h__

#if defined(_MSC_VER) && (_MSC_VER >= 1020)
#pragma once
#endif

#ifndef DECLSPEC_XFGVIRT
#if defined(_CONTROL_FLOW_GUARD_XFG)
#define DECLSPEC_XFGVIRT(base, func) __declspec(xfg_virtual(base, func))
#else
#define DECLSPEC_XFGVIRT(base, func)
#endif
#endif

/* Forward Declarations */ 

#ifndef ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_FWD_DEFINED__
#define ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_FWD_DEFINED__
typedef interface __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream;

#endif 	/* ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_FWD_DEFINED__ */


#ifndef ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_FWD_DEFINED__
#define ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_FWD_DEFINED__
typedef interface __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream;

#endif 	/* ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_FWD_DEFINED__ */


#ifndef ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_FWD_DEFINED__
#define ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_FWD_DEFINED__
typedef interface __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs;

#endif 	/* ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransport __x_ABI_CPbk20191_CWinAsync_CComio_CITransport;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FWD_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__
typedef interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector;

#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FWD_DEFINED__ */


/* header files for imported files */
#include "inspectable.h"
#include "AsyncInfo.h"
#include "EventToken.h"
#include "Windows.Foundation.h"
#include "Windows.Storage.Streams.h"

#ifdef __cplusplus
extern "C"{
#endif 


/* interface __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0000 */
/* [local] */ 






























extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0000_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0000_v0_0_s_ifspec;

/* interface __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2425 */




extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2425_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2425_v0_0_s_ifspec;

/* interface __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0001 */
/* [local] */ 

#ifndef DEF___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream
#define DEF___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream
#if !defined(__cplusplus) || defined(RO_NO_TEMPLATE_NAME)



extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0001_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0001_v0_0_s_ifspec;

#ifndef ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__
#define ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__

/* interface __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream */
/* [unique][uuid][object] */ 


EXTERN_C const IID IID___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("a464a93a-1a22-50c6-b3f6-c488a5f5f893")
    __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream : public IUnknown
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE Invoke( 
            /* [in] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream *asyncInfo,
            /* [in] */ AsyncStatus status) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);
        
        DECLSPEC_XFGVIRT(__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream, Invoke)
        HRESULT ( STDMETHODCALLTYPE *Invoke )( 
            __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [in] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream *asyncInfo,
            /* [in] */ AsyncStatus status);
        
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


#endif 	/* C style interface */




#endif 	/* ____FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__ */


/* interface __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0002 */
/* [local] */ 

#endif /* pinterface */
#endif /* DEF___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream */


extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0002_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0002_v0_0_s_ifspec;

/* interface __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2426 */




extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2426_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2426_v0_0_s_ifspec;

/* interface __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0003 */
/* [local] */ 

#ifndef DEF___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream
#define DEF___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream
#if !defined(__cplusplus) || defined(RO_NO_TEMPLATE_NAME)


extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0003_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0003_v0_0_s_ifspec;

#ifndef ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__
#define ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__

/* interface __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream */
/* [unique][uuid][object] */ 


EXTERN_C const IID IID___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("f68136b7-f950-5f98-9399-310a0df73ad6")
    __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream : public IInspectable
    {
    public:
        virtual /* [propput] */ HRESULT STDMETHODCALLTYPE put_Completed( 
            /* [in] */ __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream *handler) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_Completed( 
            /* [retval][out] */ __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream **handler) = 0;
        
        virtual HRESULT STDMETHODCALLTYPE GetResults( 
            /* [retval][out] */ __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream **results) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream, put_Completed)
        /* [propput] */ HRESULT ( STDMETHODCALLTYPE *put_Completed )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [in] */ __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream *handler);
        
        DECLSPEC_XFGVIRT(__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream, get_Completed)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_Completed )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [retval][out] */ __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream **handler);
        
        DECLSPEC_XFGVIRT(__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream, GetResults)
        HRESULT ( STDMETHODCALLTYPE *GetResults )( 
            __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream * This,
            /* [retval][out] */ __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream **results);
        
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


#endif 	/* C style interface */




#endif 	/* ____FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_INTERFACE_DEFINED__ */


/* interface __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0004 */
/* [local] */ 

#endif /* pinterface */
#endif /* DEF___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream */


extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0004_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0004_v0_0_s_ifspec;

/* interface __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2427 */




extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2427_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2427_v0_0_s_ifspec;

/* interface __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0005 */
/* [local] */ 

#ifndef DEF___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs
#define DEF___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs
#if !defined(__cplusplus) || defined(RO_NO_TEMPLATE_NAME)


extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0005_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0005_v0_0_s_ifspec;

#ifndef ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_INTERFACE_DEFINED__
#define ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_INTERFACE_DEFINED__

/* interface __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs */
/* [unique][uuid][object] */ 


EXTERN_C const IID IID___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("0d47445c-a1be-507c-8ca9-d728a0fe89d8")
    __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs : public IUnknown
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE Invoke( 
            /* [in] */ __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener *sender,
            /* [in] */ __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs *e) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This);
        
        DECLSPEC_XFGVIRT(__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs, Invoke)
        HRESULT ( STDMETHODCALLTYPE *Invoke )( 
            __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs * This,
            /* [in] */ __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener *sender,
            /* [in] */ __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs *e);
        
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


#endif 	/* C style interface */




#endif 	/* ____FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_INTERFACE_DEFINED__ */


/* interface __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0006 */
/* [local] */ 

#endif /* pinterface */
#endif /* DEF___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs */


extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0006_v0_0_c_ifspec;
extern RPC_IF_HANDLE __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0006_v0_0_s_ifspec;

#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("D0E8FF19-BA8B-5263-9551-0F052DADA51B")
    __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener : public IInspectable
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE add_ConnectionReceived( 
            /* [in] */ __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs *handler,
            /* [out][retval] */ EventRegistrationToken *__returnValue_add_ConnectionReceived__) = 0;
        
        virtual HRESULT STDMETHODCALLTYPE remove_ConnectionReceived( 
            /* [in] */ EventRegistrationToken token) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_Information( 
            /* [out][retval] */ HSTRING *__returnValue_get_Information__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener, add_ConnectionReceived)
        HRESULT ( STDMETHODCALLTYPE *add_ConnectionReceived )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
            /* [in] */ __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs *handler,
            /* [out][retval] */ EventRegistrationToken *__returnValue_add_ConnectionReceived__);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener, remove_ConnectionReceived)
        HRESULT ( STDMETHODCALLTYPE *remove_ConnectionReceived )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
            /* [in] */ EventRegistrationToken token);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener, get_Information)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_Information )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener * This,
            /* [out][retval] */ HSTRING *__returnValue_get_Information__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_add_ConnectionReceived(This,handler,__returnValue_add_ConnectionReceived__)	\
    ( (This)->lpVtbl -> add_ConnectionReceived(This,handler,__returnValue_add_ConnectionReceived__) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_remove_ConnectionReceived(This,token)	\
    ( (This)->lpVtbl -> remove_ConnectionReceived(This,token) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_get_Information(This,__returnValue_get_Information__)	\
    ( (This)->lpVtbl -> get_Information(This,__returnValue_get_Information__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("661712AC-63E3-5243-A91A-6409D33FC02B")
    __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs : public IInspectable
    {
    public:
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_Stream( 
            /* [out][retval] */ __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream **__returnValue_get_Stream__) = 0;
        
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_RemoteEndpoint( 
            /* [out][retval] */ HSTRING *__returnValue_get_RemoteEndpoint__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs, get_Stream)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_Stream )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
            /* [out][retval] */ __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream **__returnValue_get_Stream__);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs, get_RemoteEndpoint)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_RemoteEndpoint )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs * This,
            /* [out][retval] */ HSTRING *__returnValue_get_RemoteEndpoint__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_get_Stream(This,__returnValue_get_Stream__)	\
    ( (This)->lpVtbl -> get_Stream(This,__returnValue_get_Stream__) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_get_RemoteEndpoint(This,__returnValue_get_RemoteEndpoint__)	\
    ( (This)->lpVtbl -> get_RemoteEndpoint(This,__returnValue_get_RemoteEndpoint__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("38DEC2E0-5F8F-5B4B-8CEB-E57D53390C1C")
    __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind : public IInspectable
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE BindEndpointAsync( 
            /* [in] */ HSTRING endpoint,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindEndpointAsync__) = 0;
        
        virtual HRESULT STDMETHODCALLTYPE BindServiceNameAsync( 
            /* [in] */ HSTRING serviceName,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindServiceNameAsync__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind, BindEndpointAsync)
        HRESULT ( STDMETHODCALLTYPE *BindEndpointAsync )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
            /* [in] */ HSTRING endpoint,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindEndpointAsync__);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind, BindServiceNameAsync)
        HRESULT ( STDMETHODCALLTYPE *BindServiceNameAsync )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind * This,
            /* [in] */ HSTRING serviceName,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindServiceNameAsync__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_BindEndpointAsync(This,endpoint,__returnValue_BindEndpointAsync__)	\
    ( (This)->lpVtbl -> BindEndpointAsync(This,endpoint,__returnValue_BindEndpointAsync__) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_BindServiceNameAsync(This,serviceName,__returnValue_BindServiceNameAsync__)	\
    ( (This)->lpVtbl -> BindServiceNameAsync(This,serviceName,__returnValue_BindServiceNameAsync__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("7668644D-7C82-57DC-A371-B7404F3E99B0")
    __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector : public IInspectable
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE ConnectAsync( 
            /* [in] */ HSTRING endpoint,
            /* [out][retval] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream **__returnValue_ConnectAsync__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector, ConnectAsync)
        HRESULT ( STDMETHODCALLTYPE *ConnectAsync )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector * This,
            /* [in] */ HSTRING endpoint,
            /* [out][retval] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream **__returnValue_ConnectAsync__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ConnectAsync(This,endpoint,__returnValue_ConnectAsync__)	\
    ( (This)->lpVtbl -> ConnectAsync(This,endpoint,__returnValue_ConnectAsync__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("4E5DB519-5FB8-5E37-81C8-B08A4A10B4FA")
    __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind : public IInspectable
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE BindAsync( 
            /* [in] */ HSTRING name,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindAsync__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind, BindAsync)
        HRESULT ( STDMETHODCALLTYPE *BindAsync )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind * This,
            /* [in] */ HSTRING name,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindAsync__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_BindAsync(This,name,__returnValue_BindAsync__)	\
    ( (This)->lpVtbl -> BindAsync(This,name,__returnValue_BindAsync__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("9AD13FB2-9C7F-5286-A9CF-71F9B4C3E08F")
    __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector : public IInspectable
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE ConnectAsync( 
            /* [in] */ HSTRING name,
            /* [out][retval] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream **__returnValue_ConnectAsync__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector, ConnectAsync)
        HRESULT ( STDMETHODCALLTYPE *ConnectAsync )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector * This,
            /* [in] */ HSTRING name,
            /* [out][retval] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream **__returnValue_ConnectAsync__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ConnectAsync(This,name,__returnValue_ConnectAsync__)	\
    ( (This)->lpVtbl -> ConnectAsync(This,name,__returnValue_ConnectAsync__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransport */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransport;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("9455E7DE-0C4F-5D23-902A-A44EBF0A4AE6")
    __x_ABI_CPbk20191_CWinAsync_CComio_CITransport : public IInspectable
    {
    public:
        virtual /* [propget] */ HRESULT STDMETHODCALLTYPE get_Transport( 
            /* [out][retval] */ HSTRING *__returnValue_get_Transport__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CITransport, get_Transport)
        /* [propget] */ HRESULT ( STDMETHODCALLTYPE *get_Transport )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransport * This,
            /* [out][retval] */ HSTRING *__returnValue_get_Transport__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CITransportVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransport
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_get_Transport(This,__returnValue_get_Transport__)	\
    ( (This)->lpVtbl -> get_Transport(This,__returnValue_get_Transport__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CITransport_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("AD413F92-1757-501D-B24A-765E6B4E883C")
    __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream : public IInspectable
    {
    public:
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream * This,
            /* [out] */ TrustLevel *trustLevel);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("70FACC3F-8F25-53F5-8BA3-9C919F1F9534")
    __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind : public IInspectable
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE BindAsync( 
            /* [in] */ HSTRING path,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindAsync__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind, BindAsync)
        HRESULT ( STDMETHODCALLTYPE *BindAsync )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind * This,
            /* [in] */ HSTRING path,
            /* [out][retval] */ __x_ABI_CWindows_CFoundation_CIAsyncAction **__returnValue_BindAsync__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_BindAsync(This,path,__returnValue_BindAsync__)	\
    ( (This)->lpVtbl -> BindAsync(This,path,__returnValue_BindAsync__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_INTERFACE_DEFINED__ */


#ifndef ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__
#define ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__

/* interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector */
/* [uuid][object] */ 


EXTERN_C const IID IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector;

#if defined(__cplusplus) && !defined(CINTERFACE)
    
    MIDL_INTERFACE("79876E3D-A244-5AA3-93FE-FF040F1522EC")
    __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector : public IInspectable
    {
    public:
        virtual HRESULT STDMETHODCALLTYPE ConnectAsync( 
            /* [in] */ HSTRING path,
            /* [out][retval] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream **__returnValue_ConnectAsync__) = 0;
        
    };
    
    
#else 	/* C style interface */

    typedef struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorVtbl
    {
        BEGIN_INTERFACE
        
        DECLSPEC_XFGVIRT(IUnknown, QueryInterface)
        HRESULT ( STDMETHODCALLTYPE *QueryInterface )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
            /* [in] */ REFIID riid,
            /* [annotation][iid_is][out] */ 
            _COM_Outptr_  void **ppvObject);
        
        DECLSPEC_XFGVIRT(IUnknown, AddRef)
        ULONG ( STDMETHODCALLTYPE *AddRef )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This);
        
        DECLSPEC_XFGVIRT(IUnknown, Release)
        ULONG ( STDMETHODCALLTYPE *Release )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This);
        
        DECLSPEC_XFGVIRT(IInspectable, GetIids)
        HRESULT ( STDMETHODCALLTYPE *GetIids )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
            /* [out] */ ULONG *iidCount,
            /* [size_is][size_is][out] */ IID **iids);
        
        DECLSPEC_XFGVIRT(IInspectable, GetRuntimeClassName)
        HRESULT ( STDMETHODCALLTYPE *GetRuntimeClassName )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
            /* [out] */ HSTRING *className);
        
        DECLSPEC_XFGVIRT(IInspectable, GetTrustLevel)
        HRESULT ( STDMETHODCALLTYPE *GetTrustLevel )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
            /* [out] */ TrustLevel *trustLevel);
        
        DECLSPEC_XFGVIRT(__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector, ConnectAsync)
        HRESULT ( STDMETHODCALLTYPE *ConnectAsync )( 
            __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector * This,
            /* [in] */ HSTRING path,
            /* [out][retval] */ __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream **__returnValue_ConnectAsync__);
        
        END_INTERFACE
    } __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorVtbl;

    interface __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector
    {
        CONST_VTBL struct __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorVtbl *lpVtbl;
    };

    

#ifdef COBJMACROS


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_QueryInterface(This,riid,ppvObject)	\
    ( (This)->lpVtbl -> QueryInterface(This,riid,ppvObject) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_AddRef(This)	\
    ( (This)->lpVtbl -> AddRef(This) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_Release(This)	\
    ( (This)->lpVtbl -> Release(This) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_GetIids(This,iidCount,iids)	\
    ( (This)->lpVtbl -> GetIids(This,iidCount,iids) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_GetRuntimeClassName(This,className)	\
    ( (This)->lpVtbl -> GetRuntimeClassName(This,className) ) 

#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_GetTrustLevel(This,trustLevel)	\
    ( (This)->lpVtbl -> GetTrustLevel(This,trustLevel) ) 


#define __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ConnectAsync(This,path,__returnValue_ConnectAsync__)	\
    ( (This)->lpVtbl -> ConnectAsync(This,path,__returnValue_ConnectAsync__) ) 

#endif /* COBJMACROS */


#endif 	/* C style interface */




#endif 	/* ____x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_INTERFACE_DEFINED__ */


/* Additional Prototypes for ALL interfaces */

unsigned long             __RPC_USER  HSTRING_UserSize(     unsigned long *, unsigned long            , HSTRING * ); 
unsigned char * __RPC_USER  HSTRING_UserMarshal(  unsigned long *, unsigned char *, HSTRING * ); 
unsigned char * __RPC_USER  HSTRING_UserUnmarshal(unsigned long *, unsigned char *, HSTRING * ); 
void                      __RPC_USER  HSTRING_UserFree(     unsigned long *, HSTRING * ); 

unsigned long             __RPC_USER  HSTRING_UserSize64(     unsigned long *, unsigned long            , HSTRING * ); 
unsigned char * __RPC_USER  HSTRING_UserMarshal64(  unsigned long *, unsigned char *, HSTRING * ); 
unsigned char * __RPC_USER  HSTRING_UserUnmarshal64(unsigned long *, unsigned char *, HSTRING * ); 
void                      __RPC_USER  HSTRING_UserFree64(     unsigned long *, HSTRING * ); 

/* end of Additional Prototypes */

#ifdef __cplusplus
}
#endif

#endif


