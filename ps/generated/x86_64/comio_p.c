

/* this ALWAYS GENERATED file contains the proxy stub code */


 /* File created by MIDL compiler version 8.01.0628 */
/* at Tue Jan 19 12:14:07 2038
 */
/* Compiler settings for Pbk20191.WinAsync.Comio.idl:
    Oicf, W1, Zp8, env=Win64 (32b run), target_arch=AMD64 8.01.0628 
    protocol : all , ms_ext, c_ext, robust
    error checks: allocation ref bounds_check enum stub_data 
    VC __declspec() decoration level: 
         __declspec(uuid()), __declspec(selectany), __declspec(novtable)
         DECLSPEC_UUID(), MIDL_INTERFACE()
*/
/* @@MIDL_FILE_HEADING(  ) */

#if defined(_M_AMD64)


#if _MSC_VER >= 1200
#pragma warning(push)
#endif

#pragma warning( disable: 4211 )  /* redefine extern to static */
#pragma warning( disable: 4232 )  /* dllimport identity*/
#pragma warning( disable: 4024 )  /* array to pointer mapping*/
#pragma warning( disable: 4152 )  /* function/data pointer conversion in expression */

#define USE_STUBLESS_PROXY


/* verify that the <rpcproxy.h> version is high enough to compile this file*/
#ifndef __REDQ_RPCPROXY_H_VERSION__
#define __REQUIRED_RPCPROXY_H_VERSION__ 475
#endif


#include "rpcproxy.h"
#include "ndr64types.h"
#ifndef __RPCPROXY_H_VERSION__
#error this stub requires an updated version of <rpcproxy.h>
#endif /* __RPCPROXY_H_VERSION__ */


#include "comio_p.h"

#define TYPE_FORMAT_STRING_SIZE   249                               
#define PROC_FORMAT_STRING_SIZE   569                               
#define EXPR_FORMAT_STRING_SIZE   1                                 
#define TRANSMIT_AS_TABLE_SIZE    0            
#define WIRE_MARSHAL_TABLE_SIZE   1            

typedef struct _Pbk201912EWinAsync2EComio_MIDL_TYPE_FORMAT_STRING
    {
    short          Pad;
    unsigned char  Format[ TYPE_FORMAT_STRING_SIZE ];
    } Pbk201912EWinAsync2EComio_MIDL_TYPE_FORMAT_STRING;

typedef struct _Pbk201912EWinAsync2EComio_MIDL_PROC_FORMAT_STRING
    {
    short          Pad;
    unsigned char  Format[ PROC_FORMAT_STRING_SIZE ];
    } Pbk201912EWinAsync2EComio_MIDL_PROC_FORMAT_STRING;

typedef struct _Pbk201912EWinAsync2EComio_MIDL_EXPR_FORMAT_STRING
    {
    long          Pad;
    unsigned char  Format[ EXPR_FORMAT_STRING_SIZE ];
    } Pbk201912EWinAsync2EComio_MIDL_EXPR_FORMAT_STRING;


static const RPC_SYNTAX_IDENTIFIER  _RpcTransferSyntax_2_0 = 
{{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}};

static const RPC_SYNTAX_IDENTIFIER  _NDR64_RpcTransferSyntax_1_0 = 
{{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}};

#if defined(_CONTROL_FLOW_GUARD_XFG)
#define XFG_TRAMPOLINES(ObjectType)\
NDR_SHAREABLE unsigned long ObjectType ## _UserSize_XFG(unsigned long * pFlags, unsigned long Offset, void * pObject)\
{\
return  ObjectType ## _UserSize(pFlags, Offset, (ObjectType *)pObject);\
}\
NDR_SHAREABLE unsigned char * ObjectType ## _UserMarshal_XFG(unsigned long * pFlags, unsigned char * pBuffer, void * pObject)\
{\
return ObjectType ## _UserMarshal(pFlags, pBuffer, (ObjectType *)pObject);\
}\
NDR_SHAREABLE unsigned char * ObjectType ## _UserUnmarshal_XFG(unsigned long * pFlags, unsigned char * pBuffer, void * pObject)\
{\
return ObjectType ## _UserUnmarshal(pFlags, pBuffer, (ObjectType *)pObject);\
}\
NDR_SHAREABLE void ObjectType ## _UserFree_XFG(unsigned long * pFlags, void * pObject)\
{\
ObjectType ## _UserFree(pFlags, (ObjectType *)pObject);\
}
#define XFG_TRAMPOLINES64(ObjectType)\
NDR_SHAREABLE unsigned long ObjectType ## _UserSize64_XFG(unsigned long * pFlags, unsigned long Offset, void * pObject)\
{\
return  ObjectType ## _UserSize64(pFlags, Offset, (ObjectType *)pObject);\
}\
NDR_SHAREABLE unsigned char * ObjectType ## _UserMarshal64_XFG(unsigned long * pFlags, unsigned char * pBuffer, void * pObject)\
{\
return ObjectType ## _UserMarshal64(pFlags, pBuffer, (ObjectType *)pObject);\
}\
NDR_SHAREABLE unsigned char * ObjectType ## _UserUnmarshal64_XFG(unsigned long * pFlags, unsigned char * pBuffer, void * pObject)\
{\
return ObjectType ## _UserUnmarshal64(pFlags, pBuffer, (ObjectType *)pObject);\
}\
NDR_SHAREABLE void ObjectType ## _UserFree64_XFG(unsigned long * pFlags, void * pObject)\
{\
ObjectType ## _UserFree64(pFlags, (ObjectType *)pObject);\
}
#define XFG_BIND_TRAMPOLINES(HandleType, ObjectType)\
static void* ObjectType ## _bind_XFG(HandleType pObject)\
{\
return ObjectType ## _bind((ObjectType) pObject);\
}\
static void ObjectType ## _unbind_XFG(HandleType pObject, handle_t ServerHandle)\
{\
ObjectType ## _unbind((ObjectType) pObject, ServerHandle);\
}
#define XFG_TRAMPOLINE_FPTR(Function) Function ## _XFG
#define XFG_TRAMPOLINE_FPTR_DEPENDENT_SYMBOL(Symbol) Symbol ## _XFG
#else
#define XFG_TRAMPOLINES(ObjectType)
#define XFG_TRAMPOLINES64(ObjectType)
#define XFG_BIND_TRAMPOLINES(HandleType, ObjectType)
#define XFG_TRAMPOLINE_FPTR(Function) Function
#define XFG_TRAMPOLINE_FPTR_DEPENDENT_SYMBOL(Symbol) Symbol
#endif



extern const Pbk201912EWinAsync2EComio_MIDL_TYPE_FORMAT_STRING Pbk201912EWinAsync2EComio__MIDL_TypeFormatString;
extern const Pbk201912EWinAsync2EComio_MIDL_PROC_FORMAT_STRING Pbk201912EWinAsync2EComio__MIDL_ProcFormatString;
extern const Pbk201912EWinAsync2EComio_MIDL_EXPR_FORMAT_STRING Pbk201912EWinAsync2EComio__MIDL_ExprFormatString;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_ProxyInfo;

#ifdef __cplusplus
namespace {
#endif

extern const MIDL_STUB_DESC Object_StubDesc;
#ifdef __cplusplus
}
#endif


extern const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ServerInfo;
extern const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ProxyInfo;


extern const USER_MARSHAL_ROUTINE_QUADRUPLE NDR64_UserMarshalRoutines[ WIRE_MARSHAL_TABLE_SIZE ];extern const USER_MARSHAL_ROUTINE_QUADRUPLE UserMarshalRoutines[ WIRE_MARSHAL_TABLE_SIZE ];

#if !defined(__RPC_WIN64__)
#error  Invalid build platform for this stub.
#endif

static const Pbk201912EWinAsync2EComio_MIDL_PROC_FORMAT_STRING Pbk201912EWinAsync2EComio__MIDL_ProcFormatString =
    {
        0,
        {

	/* Procedure Invoke */

			0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/*  2 */	NdrFcLong( 0x0 ),	/* 0 */
/*  6 */	NdrFcShort( 0x3 ),	/* 3 */
/*  8 */	NdrFcShort( 0x20 ),	/* X64 Stack size/offset = 32 */
/* 10 */	NdrFcShort( 0x8 ),	/* 8 */
/* 12 */	NdrFcShort( 0x8 ),	/* 8 */
/* 14 */	0x46,		/* Oi2 Flags:  clt must size, has return, has ext, */
			0x3,		/* 3 */
/* 16 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 18 */	NdrFcShort( 0x0 ),	/* 0 */
/* 20 */	NdrFcShort( 0x0 ),	/* 0 */
/* 22 */	NdrFcShort( 0x0 ),	/* 0 */
/* 24 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter asyncInfo */

/* 26 */	NdrFcShort( 0xb ),	/* Flags:  must size, must free, in, */
/* 28 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 30 */	NdrFcShort( 0x2 ),	/* Type Offset=2 */

	/* Parameter status */

/* 32 */	NdrFcShort( 0x48 ),	/* Flags:  in, base type, */
/* 34 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 36 */	0xe,		/* FC_ENUM32 */
			0x0,		/* 0 */

	/* Return value */

/* 38 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 40 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 42 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure put_Completed */

/* 44 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 46 */	NdrFcLong( 0x0 ),	/* 0 */
/* 50 */	NdrFcShort( 0x6 ),	/* 6 */
/* 52 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 54 */	NdrFcShort( 0x0 ),	/* 0 */
/* 56 */	NdrFcShort( 0x8 ),	/* 8 */
/* 58 */	0x46,		/* Oi2 Flags:  clt must size, has return, has ext, */
			0x2,		/* 2 */
/* 60 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 62 */	NdrFcShort( 0x0 ),	/* 0 */
/* 64 */	NdrFcShort( 0x0 ),	/* 0 */
/* 66 */	NdrFcShort( 0x0 ),	/* 0 */
/* 68 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter handler */

/* 70 */	NdrFcShort( 0xb ),	/* Flags:  must size, must free, in, */
/* 72 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 74 */	NdrFcShort( 0x14 ),	/* Type Offset=20 */

	/* Return value */

/* 76 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 78 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 80 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure get_Completed */

/* 82 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 84 */	NdrFcLong( 0x0 ),	/* 0 */
/* 88 */	NdrFcShort( 0x7 ),	/* 7 */
/* 90 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 92 */	NdrFcShort( 0x0 ),	/* 0 */
/* 94 */	NdrFcShort( 0x8 ),	/* 8 */
/* 96 */	0x45,		/* Oi2 Flags:  srv must size, has return, has ext, */
			0x2,		/* 2 */
/* 98 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 100 */	NdrFcShort( 0x0 ),	/* 0 */
/* 102 */	NdrFcShort( 0x0 ),	/* 0 */
/* 104 */	NdrFcShort( 0x0 ),	/* 0 */
/* 106 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter handler */

/* 108 */	NdrFcShort( 0x13 ),	/* Flags:  must size, must free, out, */
/* 110 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 112 */	NdrFcShort( 0x26 ),	/* Type Offset=38 */

	/* Return value */

/* 114 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 116 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 118 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure GetResults */

/* 120 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 122 */	NdrFcLong( 0x0 ),	/* 0 */
/* 126 */	NdrFcShort( 0x8 ),	/* 8 */
/* 128 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 130 */	NdrFcShort( 0x0 ),	/* 0 */
/* 132 */	NdrFcShort( 0x8 ),	/* 8 */
/* 134 */	0x45,		/* Oi2 Flags:  srv must size, has return, has ext, */
			0x2,		/* 2 */
/* 136 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 138 */	NdrFcShort( 0x0 ),	/* 0 */
/* 140 */	NdrFcShort( 0x0 ),	/* 0 */
/* 142 */	NdrFcShort( 0x0 ),	/* 0 */
/* 144 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter results */

/* 146 */	NdrFcShort( 0x13 ),	/* Flags:  must size, must free, out, */
/* 148 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 150 */	NdrFcShort( 0x2a ),	/* Type Offset=42 */

	/* Return value */

/* 152 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 154 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 156 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure Invoke */

/* 158 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 160 */	NdrFcLong( 0x0 ),	/* 0 */
/* 164 */	NdrFcShort( 0x3 ),	/* 3 */
/* 166 */	NdrFcShort( 0x20 ),	/* X64 Stack size/offset = 32 */
/* 168 */	NdrFcShort( 0x0 ),	/* 0 */
/* 170 */	NdrFcShort( 0x8 ),	/* 8 */
/* 172 */	0x46,		/* Oi2 Flags:  clt must size, has return, has ext, */
			0x3,		/* 3 */
/* 174 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 176 */	NdrFcShort( 0x0 ),	/* 0 */
/* 178 */	NdrFcShort( 0x0 ),	/* 0 */
/* 180 */	NdrFcShort( 0x0 ),	/* 0 */
/* 182 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter sender */

/* 184 */	NdrFcShort( 0xb ),	/* Flags:  must size, must free, in, */
/* 186 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 188 */	NdrFcShort( 0x40 ),	/* Type Offset=64 */

	/* Parameter e */

/* 190 */	NdrFcShort( 0xb ),	/* Flags:  must size, must free, in, */
/* 192 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 194 */	NdrFcShort( 0x52 ),	/* Type Offset=82 */

	/* Return value */

/* 196 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 198 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 200 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure add_ConnectionReceived */

/* 202 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 204 */	NdrFcLong( 0x0 ),	/* 0 */
/* 208 */	NdrFcShort( 0x6 ),	/* 6 */
/* 210 */	NdrFcShort( 0x20 ),	/* X64 Stack size/offset = 32 */
/* 212 */	NdrFcShort( 0x0 ),	/* 0 */
/* 214 */	NdrFcShort( 0x34 ),	/* 52 */
/* 216 */	0x46,		/* Oi2 Flags:  clt must size, has return, has ext, */
			0x3,		/* 3 */
/* 218 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 220 */	NdrFcShort( 0x0 ),	/* 0 */
/* 222 */	NdrFcShort( 0x0 ),	/* 0 */
/* 224 */	NdrFcShort( 0x0 ),	/* 0 */
/* 226 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter handler */

/* 228 */	NdrFcShort( 0xb ),	/* Flags:  must size, must free, in, */
/* 230 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 232 */	NdrFcShort( 0x64 ),	/* Type Offset=100 */

	/* Parameter __returnValue_add_ConnectionReceived__ */

/* 234 */	NdrFcShort( 0x2112 ),	/* Flags:  must free, out, simple ref, srv alloc size=8 */
/* 236 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 238 */	NdrFcShort( 0x7a ),	/* Type Offset=122 */

	/* Return value */

/* 240 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 242 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 244 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure remove_ConnectionReceived */

/* 246 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 248 */	NdrFcLong( 0x0 ),	/* 0 */
/* 252 */	NdrFcShort( 0x7 ),	/* 7 */
/* 254 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 256 */	NdrFcShort( 0x18 ),	/* 24 */
/* 258 */	NdrFcShort( 0x8 ),	/* 8 */
/* 260 */	0x44,		/* Oi2 Flags:  has return, has ext, */
			0x2,		/* 2 */
/* 262 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 264 */	NdrFcShort( 0x0 ),	/* 0 */
/* 266 */	NdrFcShort( 0x0 ),	/* 0 */
/* 268 */	NdrFcShort( 0x0 ),	/* 0 */
/* 270 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter token */

/* 272 */	NdrFcShort( 0x8a ),	/* Flags:  must free, in, by val, */
/* 274 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 276 */	NdrFcShort( 0x7a ),	/* Type Offset=122 */

	/* Return value */

/* 278 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 280 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 282 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure get_Information */

/* 284 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 286 */	NdrFcLong( 0x0 ),	/* 0 */
/* 290 */	NdrFcShort( 0x8 ),	/* 8 */
/* 292 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 294 */	NdrFcShort( 0x0 ),	/* 0 */
/* 296 */	NdrFcShort( 0x8 ),	/* 8 */
/* 298 */	0x45,		/* Oi2 Flags:  srv must size, has return, has ext, */
			0x2,		/* 2 */
/* 300 */	0xa,		/* 10 */
			0x3,		/* Ext Flags:  new corr desc, clt corr check, */
/* 302 */	NdrFcShort( 0x1 ),	/* 1 */
/* 304 */	NdrFcShort( 0x0 ),	/* 0 */
/* 306 */	NdrFcShort( 0x0 ),	/* 0 */
/* 308 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter __returnValue_get_Information__ */

/* 310 */	NdrFcShort( 0x2113 ),	/* Flags:  must size, must free, out, simple ref, srv alloc size=8 */
/* 312 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 314 */	NdrFcShort( 0x9e ),	/* Type Offset=158 */

	/* Return value */

/* 316 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 318 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 320 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure get_Stream */

/* 322 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 324 */	NdrFcLong( 0x0 ),	/* 0 */
/* 328 */	NdrFcShort( 0x6 ),	/* 6 */
/* 330 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 332 */	NdrFcShort( 0x0 ),	/* 0 */
/* 334 */	NdrFcShort( 0x8 ),	/* 8 */
/* 336 */	0x45,		/* Oi2 Flags:  srv must size, has return, has ext, */
			0x2,		/* 2 */
/* 338 */	0xa,		/* 10 */
			0x1,		/* Ext Flags:  new corr desc, */
/* 340 */	NdrFcShort( 0x0 ),	/* 0 */
/* 342 */	NdrFcShort( 0x0 ),	/* 0 */
/* 344 */	NdrFcShort( 0x0 ),	/* 0 */
/* 346 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter __returnValue_get_Stream__ */

/* 348 */	NdrFcShort( 0x13 ),	/* Flags:  must size, must free, out, */
/* 350 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 352 */	NdrFcShort( 0xa8 ),	/* Type Offset=168 */

	/* Return value */

/* 354 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 356 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 358 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure get_RemoteEndpoint */

/* 360 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 362 */	NdrFcLong( 0x0 ),	/* 0 */
/* 366 */	NdrFcShort( 0x7 ),	/* 7 */
/* 368 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 370 */	NdrFcShort( 0x0 ),	/* 0 */
/* 372 */	NdrFcShort( 0x8 ),	/* 8 */
/* 374 */	0x45,		/* Oi2 Flags:  srv must size, has return, has ext, */
			0x2,		/* 2 */
/* 376 */	0xa,		/* 10 */
			0x3,		/* Ext Flags:  new corr desc, clt corr check, */
/* 378 */	NdrFcShort( 0x1 ),	/* 1 */
/* 380 */	NdrFcShort( 0x0 ),	/* 0 */
/* 382 */	NdrFcShort( 0x0 ),	/* 0 */
/* 384 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter __returnValue_get_RemoteEndpoint__ */

/* 386 */	NdrFcShort( 0x2113 ),	/* Flags:  must size, must free, out, simple ref, srv alloc size=8 */
/* 388 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 390 */	NdrFcShort( 0x9e ),	/* Type Offset=158 */

	/* Return value */

/* 392 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 394 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 396 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure BindAsync */


	/* Procedure BindAsync */


	/* Procedure BindEndpointAsync */

/* 398 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 400 */	NdrFcLong( 0x0 ),	/* 0 */
/* 404 */	NdrFcShort( 0x6 ),	/* 6 */
/* 406 */	NdrFcShort( 0x20 ),	/* X64 Stack size/offset = 32 */
/* 408 */	NdrFcShort( 0x0 ),	/* 0 */
/* 410 */	NdrFcShort( 0x8 ),	/* 8 */
/* 412 */	0x47,		/* Oi2 Flags:  srv must size, clt must size, has return, has ext, */
			0x3,		/* 3 */
/* 414 */	0xa,		/* 10 */
			0x5,		/* Ext Flags:  new corr desc, srv corr check, */
/* 416 */	NdrFcShort( 0x0 ),	/* 0 */
/* 418 */	NdrFcShort( 0x1 ),	/* 1 */
/* 420 */	NdrFcShort( 0x0 ),	/* 0 */
/* 422 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter path */


	/* Parameter name */


	/* Parameter endpoint */

/* 424 */	NdrFcShort( 0x8b ),	/* Flags:  must size, must free, in, by val, */
/* 426 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 428 */	NdrFcShort( 0xc2 ),	/* Type Offset=194 */

	/* Parameter __returnValue_BindAsync__ */


	/* Parameter __returnValue_BindAsync__ */


	/* Parameter __returnValue_BindEndpointAsync__ */

/* 430 */	NdrFcShort( 0x13 ),	/* Flags:  must size, must free, out, */
/* 432 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 434 */	NdrFcShort( 0xcc ),	/* Type Offset=204 */

	/* Return value */


	/* Return value */


	/* Return value */

/* 436 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 438 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 440 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure BindServiceNameAsync */

/* 442 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 444 */	NdrFcLong( 0x0 ),	/* 0 */
/* 448 */	NdrFcShort( 0x7 ),	/* 7 */
/* 450 */	NdrFcShort( 0x20 ),	/* X64 Stack size/offset = 32 */
/* 452 */	NdrFcShort( 0x0 ),	/* 0 */
/* 454 */	NdrFcShort( 0x8 ),	/* 8 */
/* 456 */	0x47,		/* Oi2 Flags:  srv must size, clt must size, has return, has ext, */
			0x3,		/* 3 */
/* 458 */	0xa,		/* 10 */
			0x5,		/* Ext Flags:  new corr desc, srv corr check, */
/* 460 */	NdrFcShort( 0x0 ),	/* 0 */
/* 462 */	NdrFcShort( 0x1 ),	/* 1 */
/* 464 */	NdrFcShort( 0x0 ),	/* 0 */
/* 466 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter serviceName */

/* 468 */	NdrFcShort( 0x8b ),	/* Flags:  must size, must free, in, by val, */
/* 470 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 472 */	NdrFcShort( 0xc2 ),	/* Type Offset=194 */

	/* Parameter __returnValue_BindServiceNameAsync__ */

/* 474 */	NdrFcShort( 0x13 ),	/* Flags:  must size, must free, out, */
/* 476 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 478 */	NdrFcShort( 0xcc ),	/* Type Offset=204 */

	/* Return value */

/* 480 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 482 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 484 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure ConnectAsync */


	/* Procedure ConnectAsync */


	/* Procedure ConnectAsync */

/* 486 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 488 */	NdrFcLong( 0x0 ),	/* 0 */
/* 492 */	NdrFcShort( 0x6 ),	/* 6 */
/* 494 */	NdrFcShort( 0x20 ),	/* X64 Stack size/offset = 32 */
/* 496 */	NdrFcShort( 0x0 ),	/* 0 */
/* 498 */	NdrFcShort( 0x8 ),	/* 8 */
/* 500 */	0x47,		/* Oi2 Flags:  srv must size, clt must size, has return, has ext, */
			0x3,		/* 3 */
/* 502 */	0xa,		/* 10 */
			0x5,		/* Ext Flags:  new corr desc, srv corr check, */
/* 504 */	NdrFcShort( 0x0 ),	/* 0 */
/* 506 */	NdrFcShort( 0x1 ),	/* 1 */
/* 508 */	NdrFcShort( 0x0 ),	/* 0 */
/* 510 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter path */


	/* Parameter name */


	/* Parameter endpoint */

/* 512 */	NdrFcShort( 0x8b ),	/* Flags:  must size, must free, in, by val, */
/* 514 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 516 */	NdrFcShort( 0xc2 ),	/* Type Offset=194 */

	/* Parameter __returnValue_ConnectAsync__ */


	/* Parameter __returnValue_ConnectAsync__ */


	/* Parameter __returnValue_ConnectAsync__ */

/* 518 */	NdrFcShort( 0x13 ),	/* Flags:  must size, must free, out, */
/* 520 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 522 */	NdrFcShort( 0xe2 ),	/* Type Offset=226 */

	/* Return value */


	/* Return value */


	/* Return value */

/* 524 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 526 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 528 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

	/* Procedure get_Transport */

/* 530 */	0x33,		/* FC_AUTO_HANDLE */
			0x6c,		/* Old Flags:  object, Oi2 */
/* 532 */	NdrFcLong( 0x0 ),	/* 0 */
/* 536 */	NdrFcShort( 0x6 ),	/* 6 */
/* 538 */	NdrFcShort( 0x18 ),	/* X64 Stack size/offset = 24 */
/* 540 */	NdrFcShort( 0x0 ),	/* 0 */
/* 542 */	NdrFcShort( 0x8 ),	/* 8 */
/* 544 */	0x45,		/* Oi2 Flags:  srv must size, has return, has ext, */
			0x2,		/* 2 */
/* 546 */	0xa,		/* 10 */
			0x3,		/* Ext Flags:  new corr desc, clt corr check, */
/* 548 */	NdrFcShort( 0x1 ),	/* 1 */
/* 550 */	NdrFcShort( 0x0 ),	/* 0 */
/* 552 */	NdrFcShort( 0x0 ),	/* 0 */
/* 554 */	NdrFcShort( 0x0 ),	/* 0 */

	/* Parameter __returnValue_get_Transport__ */

/* 556 */	NdrFcShort( 0x2113 ),	/* Flags:  must size, must free, out, simple ref, srv alloc size=8 */
/* 558 */	NdrFcShort( 0x8 ),	/* X64 Stack size/offset = 8 */
/* 560 */	NdrFcShort( 0x9e ),	/* Type Offset=158 */

	/* Return value */

/* 562 */	NdrFcShort( 0x70 ),	/* Flags:  out, return, base type, */
/* 564 */	NdrFcShort( 0x10 ),	/* X64 Stack size/offset = 16 */
/* 566 */	0x8,		/* FC_LONG */
			0x0,		/* 0 */

			0x0
        }
    };

static const Pbk201912EWinAsync2EComio_MIDL_TYPE_FORMAT_STRING Pbk201912EWinAsync2EComio__MIDL_TypeFormatString =
    {
        0,
        {
			NdrFcShort( 0x0 ),	/* 0 */
/*  2 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/*  4 */	NdrFcLong( 0xf68136b7 ),	/* -159304009 */
/*  8 */	NdrFcShort( 0xf950 ),	/* -1712 */
/* 10 */	NdrFcShort( 0x5f98 ),	/* 24472 */
/* 12 */	0x93,		/* 147 */
			0x99,		/* 153 */
/* 14 */	0x31,		/* 49 */
			0xa,		/* 10 */
/* 16 */	0xd,		/* 13 */
			0xf7,		/* 247 */
/* 18 */	0x3a,		/* 58 */
			0xd6,		/* 214 */
/* 20 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 22 */	NdrFcLong( 0xa464a93a ),	/* -1536906950 */
/* 26 */	NdrFcShort( 0x1a22 ),	/* 6690 */
/* 28 */	NdrFcShort( 0x50c6 ),	/* 20678 */
/* 30 */	0xb3,		/* 179 */
			0xf6,		/* 246 */
/* 32 */	0xc4,		/* 196 */
			0x88,		/* 136 */
/* 34 */	0xa5,		/* 165 */
			0xf5,		/* 245 */
/* 36 */	0xf8,		/* 248 */
			0x93,		/* 147 */
/* 38 */	
			0x11, 0x10,	/* FC_RP [pointer_deref] */
/* 40 */	NdrFcShort( 0xffec ),	/* Offset= -20 (20) */
/* 42 */	
			0x11, 0x10,	/* FC_RP [pointer_deref] */
/* 44 */	NdrFcShort( 0x2 ),	/* Offset= 2 (46) */
/* 46 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 48 */	NdrFcLong( 0xad413f92 ),	/* -1388232814 */
/* 52 */	NdrFcShort( 0x1757 ),	/* 5975 */
/* 54 */	NdrFcShort( 0x501d ),	/* 20509 */
/* 56 */	0xb2,		/* 178 */
			0x4a,		/* 74 */
/* 58 */	0x76,		/* 118 */
			0x5e,		/* 94 */
/* 60 */	0x6b,		/* 107 */
			0x4e,		/* 78 */
/* 62 */	0x88,		/* 136 */
			0x3c,		/* 60 */
/* 64 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 66 */	NdrFcLong( 0xd0e8ff19 ),	/* -790036711 */
/* 70 */	NdrFcShort( 0xba8b ),	/* -17781 */
/* 72 */	NdrFcShort( 0x5263 ),	/* 21091 */
/* 74 */	0x95,		/* 149 */
			0x51,		/* 81 */
/* 76 */	0xf,		/* 15 */
			0x5,		/* 5 */
/* 78 */	0x2d,		/* 45 */
			0xad,		/* 173 */
/* 80 */	0xa5,		/* 165 */
			0x1b,		/* 27 */
/* 82 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 84 */	NdrFcLong( 0x661712ac ),	/* 1712788140 */
/* 88 */	NdrFcShort( 0x63e3 ),	/* 25571 */
/* 90 */	NdrFcShort( 0x5243 ),	/* 21059 */
/* 92 */	0xa9,		/* 169 */
			0x1a,		/* 26 */
/* 94 */	0x64,		/* 100 */
			0x9,		/* 9 */
/* 96 */	0xd3,		/* 211 */
			0x3f,		/* 63 */
/* 98 */	0xc0,		/* 192 */
			0x2b,		/* 43 */
/* 100 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 102 */	NdrFcLong( 0xd47445c ),	/* 222774364 */
/* 106 */	NdrFcShort( 0xa1be ),	/* -24130 */
/* 108 */	NdrFcShort( 0x507c ),	/* 20604 */
/* 110 */	0x8c,		/* 140 */
			0xa9,		/* 169 */
/* 112 */	0xd7,		/* 215 */
			0x28,		/* 40 */
/* 114 */	0xa0,		/* 160 */
			0xfe,		/* 254 */
/* 116 */	0x89,		/* 137 */
			0xd8,		/* 216 */
/* 118 */	
			0x11, 0x4,	/* FC_RP [alloced_on_stack] */
/* 120 */	NdrFcShort( 0x2 ),	/* Offset= 2 (122) */
/* 122 */	
			0x15,		/* FC_STRUCT */
			0x7,		/* 7 */
/* 124 */	NdrFcShort( 0x8 ),	/* 8 */
/* 126 */	0xb,		/* FC_HYPER */
			0x5b,		/* FC_END */
/* 128 */	
			0x11, 0x4,	/* FC_RP [alloced_on_stack] */
/* 130 */	NdrFcShort( 0x1c ),	/* Offset= 28 (158) */
/* 132 */	
			0x13, 0x0,	/* FC_OP */
/* 134 */	NdrFcShort( 0xe ),	/* Offset= 14 (148) */
/* 136 */	
			0x1b,		/* FC_CARRAY */
			0x1,		/* 1 */
/* 138 */	NdrFcShort( 0x2 ),	/* 2 */
/* 140 */	0x9,		/* Corr desc: FC_ULONG */
			0x0,		/*  */
/* 142 */	NdrFcShort( 0xfffc ),	/* -4 */
/* 144 */	NdrFcShort( 0x1 ),	/* Corr flags:  early, */
/* 146 */	0x6,		/* FC_SHORT */
			0x5b,		/* FC_END */
/* 148 */	
			0x17,		/* FC_CSTRUCT */
			0x3,		/* 3 */
/* 150 */	NdrFcShort( 0x8 ),	/* 8 */
/* 152 */	NdrFcShort( 0xfff0 ),	/* Offset= -16 (136) */
/* 154 */	0x8,		/* FC_LONG */
			0x8,		/* FC_LONG */
/* 156 */	0x5c,		/* FC_PAD */
			0x5b,		/* FC_END */
/* 158 */	0xb4,		/* FC_USER_MARSHAL */
			0x83,		/* 131 */
/* 160 */	NdrFcShort( 0x0 ),	/* 0 */
/* 162 */	NdrFcShort( 0x8 ),	/* 8 */
/* 164 */	NdrFcShort( 0x0 ),	/* 0 */
/* 166 */	NdrFcShort( 0xffde ),	/* Offset= -34 (132) */
/* 168 */	
			0x11, 0x10,	/* FC_RP [pointer_deref] */
/* 170 */	NdrFcShort( 0x2 ),	/* Offset= 2 (172) */
/* 172 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 174 */	NdrFcLong( 0xad413f92 ),	/* -1388232814 */
/* 178 */	NdrFcShort( 0x1757 ),	/* 5975 */
/* 180 */	NdrFcShort( 0x501d ),	/* 20509 */
/* 182 */	0xb2,		/* 178 */
			0x4a,		/* 74 */
/* 184 */	0x76,		/* 118 */
			0x5e,		/* 94 */
/* 186 */	0x6b,		/* 107 */
			0x4e,		/* 78 */
/* 188 */	0x88,		/* 136 */
			0x3c,		/* 60 */
/* 190 */	
			0x12, 0x0,	/* FC_UP */
/* 192 */	NdrFcShort( 0xffd4 ),	/* Offset= -44 (148) */
/* 194 */	0xb4,		/* FC_USER_MARSHAL */
			0x83,		/* 131 */
/* 196 */	NdrFcShort( 0x0 ),	/* 0 */
/* 198 */	NdrFcShort( 0x8 ),	/* 8 */
/* 200 */	NdrFcShort( 0x0 ),	/* 0 */
/* 202 */	NdrFcShort( 0xfff4 ),	/* Offset= -12 (190) */
/* 204 */	
			0x11, 0x10,	/* FC_RP [pointer_deref] */
/* 206 */	NdrFcShort( 0x2 ),	/* Offset= 2 (208) */
/* 208 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 210 */	NdrFcLong( 0x5a648006 ),	/* 1516535814 */
/* 214 */	NdrFcShort( 0x843a ),	/* -31686 */
/* 216 */	NdrFcShort( 0x4da9 ),	/* 19881 */
/* 218 */	0x86,		/* 134 */
			0x5b,		/* 91 */
/* 220 */	0x9d,		/* 157 */
			0x26,		/* 38 */
/* 222 */	0xe5,		/* 229 */
			0xdf,		/* 223 */
/* 224 */	0xad,		/* 173 */
			0x7b,		/* 123 */
/* 226 */	
			0x11, 0x10,	/* FC_RP [pointer_deref] */
/* 228 */	NdrFcShort( 0x2 ),	/* Offset= 2 (230) */
/* 230 */	
			0x2f,		/* FC_IP */
			0x5a,		/* FC_CONSTANT_IID */
/* 232 */	NdrFcLong( 0xf68136b7 ),	/* -159304009 */
/* 236 */	NdrFcShort( 0xf950 ),	/* -1712 */
/* 238 */	NdrFcShort( 0x5f98 ),	/* 24472 */
/* 240 */	0x93,		/* 147 */
			0x99,		/* 153 */
/* 242 */	0x31,		/* 49 */
			0xa,		/* 10 */
/* 244 */	0xd,		/* 13 */
			0xf7,		/* 247 */
/* 246 */	0x3a,		/* 58 */
			0xd6,		/* 214 */

			0x0
        }
    };

XFG_TRAMPOLINES(HSTRING)

static const USER_MARSHAL_ROUTINE_QUADRUPLE UserMarshalRoutines[ WIRE_MARSHAL_TABLE_SIZE ] = 
        {
            
            {
            (USER_MARSHAL_SIZING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserSize)
            ,(USER_MARSHAL_MARSHALLING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserMarshal)
            ,(USER_MARSHAL_UNMARSHALLING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserUnmarshal)
            ,(USER_MARSHAL_FREEING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserFree)
            
            }
            

        };



/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0000, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2425, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0001, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: IUnknown, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0xC0,0x00,0x00,0x00,0x00,0x00,0x00,0x46}} */


/* Object interface: __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream, ver. 0.0,
   GUID={0xa464a93a,0x1a22,0x50c6,{0xb3,0xf6,0xc4,0x88,0xa5,0xf5,0xf8,0x93}} */

#pragma code_seg(".orpc")
static const unsigned short __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[] =
    {
    0
    };



/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0002, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2426, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0003, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: IInspectable, ver. 0.0,
   GUID={0xAF86E2E0,0xB12D,0x4c6a,{0x9C,0x5A,0xD7,0xAA,0x65,0x10,0x1E,0x90}} */


/* Object interface: __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream, ver. 0.0,
   GUID={0xf68136b7,0xf950,0x5f98,{0x93,0x99,0x31,0x0a,0x0d,0xf7,0x3a,0xd6}} */

#pragma code_seg(".orpc")
static const unsigned short __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    44,
    82,
    120
    };



/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0004, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2427, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0005, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs, ver. 0.0,
   GUID={0x0d47445c,0xa1be,0x507c,{0x8c,0xa9,0xd7,0x28,0xa0,0xfe,0x89,0xd8}} */

#pragma code_seg(".orpc")
static const unsigned short __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_FormatStringOffsetTable[] =
    {
    158
    };



/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0006, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener, ver. 0.0,
   GUID={0xD0E8FF19,0xBA8B,0x5263,{0x95,0x51,0x0F,0x05,0x2D,0xAD,0xA5,0x1B}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    202,
    246,
    284
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs, ver. 0.0,
   GUID={0x661712AC,0x63E3,0x5243,{0xA9,0x1A,0x64,0x09,0xD3,0x3F,0xC0,0x2B}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    322,
    360
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind, ver. 0.0,
   GUID={0x38DEC2E0,0x5F8F,0x5B4B,{0x8C,0xEB,0xE5,0x7D,0x53,0x39,0x0C,0x1C}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    398,
    442
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector, ver. 0.0,
   GUID={0x7668644D,0x7C82,0x57DC,{0xA3,0x71,0xB7,0x40,0x4F,0x3E,0x99,0xB0}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    486
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind, ver. 0.0,
   GUID={0x4E5DB519,0x5FB8,0x5E37,{0x81,0xC8,0xB0,0x8A,0x4A,0x10,0xB4,0xFA}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    398
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector, ver. 0.0,
   GUID={0x9AD13FB2,0x9C7F,0x5286,{0xA9,0xCF,0x71,0xF9,0xB4,0xC3,0xE0,0x8F}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    486
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CITransport, ver. 0.0,
   GUID={0x9455E7DE,0x0C4F,0x5D23,{0x90,0x2A,0xA4,0x4E,0xBF,0x0A,0x4A,0xE6}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    530
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream, ver. 0.0,
   GUID={0xAD413F92,0x1757,0x501D,{0xB2,0x4A,0x76,0x5E,0x6B,0x4E,0x88,0x3C}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    0
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind, ver. 0.0,
   GUID={0x70FACC3F,0x8F25,0x53F5,{0x8B,0xA3,0x9C,0x91,0x9F,0x1F,0x95,0x34}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    398
    };



/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector, ver. 0.0,
   GUID={0x79876E3D,0xA244,0x5AA3,{0x93,0xFE,0xFF,0x04,0x0F,0x15,0x22,0xEC}} */

#pragma code_seg(".orpc")
static const unsigned short __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FormatStringOffsetTable[] =
    {
    (unsigned short) -1,
    (unsigned short) -1,
    (unsigned short) -1,
    486
    };



#endif /* defined(_M_AMD64)*/



/* this ALWAYS GENERATED file contains the proxy stub code */


 /* File created by MIDL compiler version 8.01.0628 */
/* at Tue Jan 19 12:14:07 2038
 */
/* Compiler settings for Pbk20191.WinAsync.Comio.idl:
    Oicf, W1, Zp8, env=Win64 (32b run), target_arch=AMD64 8.01.0628 
    protocol : all , ms_ext, c_ext, robust
    error checks: allocation ref bounds_check enum stub_data 
    VC __declspec() decoration level: 
         __declspec(uuid()), __declspec(selectany), __declspec(novtable)
         DECLSPEC_UUID(), MIDL_INTERFACE()
*/
/* @@MIDL_FILE_HEADING(  ) */

#if defined(_M_AMD64)



extern const USER_MARSHAL_ROUTINE_QUADRUPLE NDR64_UserMarshalRoutines[ WIRE_MARSHAL_TABLE_SIZE ];extern const USER_MARSHAL_ROUTINE_QUADRUPLE UserMarshalRoutines[ WIRE_MARSHAL_TABLE_SIZE ];

#if !defined(__RPC_WIN64__)
#error  Invalid build platform for this stub.
#endif


#include "ndr64types.h"
#include "pshpack8.h"
#ifdef __cplusplus
namespace {
#endif


typedef 
NDR64_FORMAT_CHAR
__midl_frag107_t;
extern const __midl_frag107_t __midl_frag107;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag106_t;
extern const __midl_frag106_t __midl_frag106;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag105_t;
extern const __midl_frag105_t __midl_frag105;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag104_t;
extern const __midl_frag104_t __midl_frag104;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag103_t;
extern const __midl_frag103_t __midl_frag103;

typedef 
struct _NDR64_USER_MARSHAL_FORMAT
__midl_frag102_t;
extern const __midl_frag102_t __midl_frag102;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
    struct _NDR64_PARAM_FORMAT frag4;
}
__midl_frag101_t;
extern const __midl_frag101_t __midl_frag101;

typedef 
NDR64_FORMAT_CHAR
__midl_frag100_t;
extern const __midl_frag100_t __midl_frag100;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag99_t;
extern const __midl_frag99_t __midl_frag99;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag98_t;
extern const __midl_frag98_t __midl_frag98;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag97_t;
extern const __midl_frag97_t __midl_frag97;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag96_t;
extern const __midl_frag96_t __midl_frag96;

typedef 
struct _NDR64_USER_MARSHAL_FORMAT
__midl_frag95_t;
extern const __midl_frag95_t __midl_frag95;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
    struct _NDR64_PARAM_FORMAT frag4;
}
__midl_frag94_t;
extern const __midl_frag94_t __midl_frag94;

typedef 
NDR64_FORMAT_CHAR
__midl_frag93_t;
extern const __midl_frag93_t __midl_frag93;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag92_t;
extern const __midl_frag92_t __midl_frag92;

typedef 
struct _NDR64_USER_MARSHAL_FORMAT
__midl_frag91_t;
extern const __midl_frag91_t __midl_frag91;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag90_t;
extern const __midl_frag90_t __midl_frag90;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
}
__midl_frag89_t;
extern const __midl_frag89_t __midl_frag89;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag87_t;
extern const __midl_frag87_t __midl_frag87;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag86_t;
extern const __midl_frag86_t __midl_frag86;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag85_t;
extern const __midl_frag85_t __midl_frag85;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
    struct _NDR64_PARAM_FORMAT frag4;
}
__midl_frag82_t;
extern const __midl_frag82_t __midl_frag82;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
    struct _NDR64_PARAM_FORMAT frag4;
}
__midl_frag75_t;
extern const __midl_frag75_t __midl_frag75;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag47_t;
extern const __midl_frag47_t __midl_frag47;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag46_t;
extern const __midl_frag46_t __midl_frag46;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag45_t;
extern const __midl_frag45_t __midl_frag45;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
}
__midl_frag44_t;
extern const __midl_frag44_t __midl_frag44;

typedef 
NDR64_FORMAT_CHAR
__midl_frag42_t;
extern const __midl_frag42_t __midl_frag42;

typedef 
struct 
{
    NDR64_FORMAT_UINT32 frag1;
    struct _NDR64_EXPR_VAR frag2;
}
__midl_frag41_t;
extern const __midl_frag41_t __midl_frag41;

typedef 
struct 
{
    struct _NDR64_CONF_ARRAY_HEADER_FORMAT frag1;
    struct _NDR64_ARRAY_ELEMENT_INFO frag2;
}
__midl_frag40_t;
extern const __midl_frag40_t __midl_frag40;

typedef 
struct 
{
    struct _NDR64_CONF_STRUCTURE_HEADER_FORMAT frag1;
}
__midl_frag39_t;
extern const __midl_frag39_t __midl_frag39;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
}
__midl_frag35_t;
extern const __midl_frag35_t __midl_frag35;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
}
__midl_frag33_t;
extern const __midl_frag33_t __midl_frag33;

typedef 
struct 
{
    struct _NDR64_STRUCTURE_HEADER_FORMAT frag1;
}
__midl_frag31_t;
extern const __midl_frag31_t __midl_frag31;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag30_t;
extern const __midl_frag30_t __midl_frag30;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag29_t;
extern const __midl_frag29_t __midl_frag29;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag28_t;
extern const __midl_frag28_t __midl_frag28;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
    struct _NDR64_PARAM_FORMAT frag4;
}
__midl_frag27_t;
extern const __midl_frag27_t __midl_frag27;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag25_t;
extern const __midl_frag25_t __midl_frag25;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag24_t;
extern const __midl_frag24_t __midl_frag24;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag23_t;
extern const __midl_frag23_t __midl_frag23;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag22_t;
extern const __midl_frag22_t __midl_frag22;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
    struct _NDR64_PARAM_FORMAT frag4;
}
__midl_frag21_t;
extern const __midl_frag21_t __midl_frag21;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
}
__midl_frag16_t;
extern const __midl_frag16_t __midl_frag16;

typedef 
struct _NDR64_CONSTANT_IID_FORMAT
__midl_frag14_t;
extern const __midl_frag14_t __midl_frag14;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag13_t;
extern const __midl_frag13_t __midl_frag13;

typedef 
struct _NDR64_POINTER_FORMAT
__midl_frag12_t;
extern const __midl_frag12_t __midl_frag12;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
}
__midl_frag11_t;
extern const __midl_frag11_t __midl_frag11;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
}
__midl_frag7_t;
extern const __midl_frag7_t __midl_frag7;

typedef 
struct 
{
    struct _NDR64_PROC_FORMAT frag1;
    struct _NDR64_PARAM_FORMAT frag2;
    struct _NDR64_PARAM_FORMAT frag3;
    struct _NDR64_PARAM_FORMAT frag4;
}
__midl_frag2_t;
extern const __midl_frag2_t __midl_frag2;

typedef 
NDR64_FORMAT_UINT32
__midl_frag1_t;
extern const __midl_frag1_t __midl_frag1;

static const __midl_frag107_t __midl_frag107 =
0x5    /* FC64_INT32 */;

static const __midl_frag106_t __midl_frag106 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0xf68136b7,
        0xf950,
        0x5f98,
        {0x93, 0x99, 0x31, 0x0a, 0x0d, 0xf7, 0x3a, 0xd6}
    }
};

static const __midl_frag105_t __midl_frag105 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag106
};

static const __midl_frag104_t __midl_frag104 =
{ 
/* **struct _NDR64_POINTER_FORMAT */
    0x20,    /* FC64_RP */
    (NDR64_UINT8) 16 /* 0x10 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag105
};

static const __midl_frag103_t __midl_frag103 =
{ 
/* *FLAGGED_WORD_BLOB */
    0x21,    /* FC64_UP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag39
};

static const __midl_frag102_t __midl_frag102 =
{ 
/* wireBSTR */
    0xa2,    /* FC64_USER_MARSHAL */
    (NDR64_UINT8) 128 /* 0x80 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    (NDR64_UINT16) 7 /* 0x7 */,
    (NDR64_UINT16) 8 /* 0x8 */,
    (NDR64_UINT32) 8 /* 0x8 */,
    (NDR64_UINT32) 0 /* 0x0 */,
    &__midl_frag103
};

static const __midl_frag101_t __midl_frag101 =
{ 
/* ConnectAsync */
    { 
    /* ConnectAsync */      /* procedure ConnectAsync */
        (NDR64_UINT32) 3014979 /* 0x2e0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, ClientMustSize, HasReturn, ServerCorrelation */
        (NDR64_UINT32) 32 /* 0x20 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 3 /* 0x3 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* path */      /* parameter path */
        &__midl_frag102,
        { 
        /* path */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in], ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* __returnValue_ConnectAsync__ */      /* parameter __returnValue_ConnectAsync__ */
        &__midl_frag104,
        { 
        /* __returnValue_ConnectAsync__ */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [out] */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag107,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        24 /* 0x18 */,   /* Stack offset */
    }
};

static const __midl_frag100_t __midl_frag100 =
0x5    /* FC64_INT32 */;

static const __midl_frag99_t __midl_frag99 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0x5a648006,
        0x843a,
        0x4da9,
        {0x86, 0x5b, 0x9d, 0x26, 0xe5, 0xdf, 0xad, 0x7b}
    }
};

static const __midl_frag98_t __midl_frag98 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag99
};

static const __midl_frag97_t __midl_frag97 =
{ 
/* **struct _NDR64_POINTER_FORMAT */
    0x20,    /* FC64_RP */
    (NDR64_UINT8) 16 /* 0x10 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag98
};

static const __midl_frag96_t __midl_frag96 =
{ 
/* *FLAGGED_WORD_BLOB */
    0x21,    /* FC64_UP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag39
};

static const __midl_frag95_t __midl_frag95 =
{ 
/* wireBSTR */
    0xa2,    /* FC64_USER_MARSHAL */
    (NDR64_UINT8) 128 /* 0x80 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    (NDR64_UINT16) 7 /* 0x7 */,
    (NDR64_UINT16) 8 /* 0x8 */,
    (NDR64_UINT32) 8 /* 0x8 */,
    (NDR64_UINT32) 0 /* 0x0 */,
    &__midl_frag96
};

static const __midl_frag94_t __midl_frag94 =
{ 
/* BindAsync */
    { 
    /* BindAsync */      /* procedure BindAsync */
        (NDR64_UINT32) 3014979 /* 0x2e0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, ClientMustSize, HasReturn, ServerCorrelation */
        (NDR64_UINT32) 32 /* 0x20 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 3 /* 0x3 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* path */      /* parameter path */
        &__midl_frag95,
        { 
        /* path */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in], ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* __returnValue_BindAsync__ */      /* parameter __returnValue_BindAsync__ */
        &__midl_frag97,
        { 
        /* __returnValue_BindAsync__ */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [out] */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag100,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        24 /* 0x18 */,   /* Stack offset */
    }
};

static const __midl_frag93_t __midl_frag93 =
0x5    /* FC64_INT32 */;

static const __midl_frag92_t __midl_frag92 =
{ 
/* *FLAGGED_WORD_BLOB */
    0x22,    /* FC64_OP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag39
};

static const __midl_frag91_t __midl_frag91 =
{ 
/* wireBSTR */
    0xa2,    /* FC64_USER_MARSHAL */
    (NDR64_UINT8) 128 /* 0x80 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    (NDR64_UINT16) 7 /* 0x7 */,
    (NDR64_UINT16) 8 /* 0x8 */,
    (NDR64_UINT32) 8 /* 0x8 */,
    (NDR64_UINT32) 0 /* 0x0 */,
    &__midl_frag92
};

static const __midl_frag90_t __midl_frag90 =
{ 
/* *wireBSTR */
    0x20,    /* FC64_RP */
    (NDR64_UINT8) 4 /* 0x4 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag91
};

static const __midl_frag89_t __midl_frag89 =
{ 
/* get_Transport */
    { 
    /* get_Transport */      /* procedure get_Transport */
        (NDR64_UINT32) 4849987 /* 0x4a0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, HasReturn, ClientCorrelation */
        (NDR64_UINT32) 24 /* 0x18 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 2 /* 0x2 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* __returnValue_get_Transport__ */      /* parameter __returnValue_get_Transport__ */
        &__midl_frag91,
        { 
        /* __returnValue_get_Transport__ */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            1
        },    /* MustSize, MustFree, [out], SimpleRef, UseCache */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    }
};

static const __midl_frag87_t __midl_frag87 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0xf68136b7,
        0xf950,
        0x5f98,
        {0x93, 0x99, 0x31, 0x0a, 0x0d, 0xf7, 0x3a, 0xd6}
    }
};

static const __midl_frag86_t __midl_frag86 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag87
};

static const __midl_frag85_t __midl_frag85 =
{ 
/* **struct _NDR64_POINTER_FORMAT */
    0x20,    /* FC64_RP */
    (NDR64_UINT8) 16 /* 0x10 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag86
};

static const __midl_frag82_t __midl_frag82 =
{ 
/* ConnectAsync */
    { 
    /* ConnectAsync */      /* procedure ConnectAsync */
        (NDR64_UINT32) 3014979 /* 0x2e0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, ClientMustSize, HasReturn, ServerCorrelation */
        (NDR64_UINT32) 32 /* 0x20 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 3 /* 0x3 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* name */      /* parameter name */
        &__midl_frag95,
        { 
        /* name */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in], ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* __returnValue_ConnectAsync__ */      /* parameter __returnValue_ConnectAsync__ */
        &__midl_frag85,
        { 
        /* __returnValue_ConnectAsync__ */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [out] */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        24 /* 0x18 */,   /* Stack offset */
    }
};

static const __midl_frag75_t __midl_frag75 =
{ 
/* BindAsync */
    { 
    /* BindAsync */      /* procedure BindAsync */
        (NDR64_UINT32) 3014979 /* 0x2e0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, ClientMustSize, HasReturn, ServerCorrelation */
        (NDR64_UINT32) 32 /* 0x20 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 3 /* 0x3 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* name */      /* parameter name */
        &__midl_frag95,
        { 
        /* name */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in], ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* __returnValue_BindAsync__ */      /* parameter __returnValue_BindAsync__ */
        &__midl_frag97,
        { 
        /* __returnValue_BindAsync__ */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [out] */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        24 /* 0x18 */,   /* Stack offset */
    }
};

static const __midl_frag47_t __midl_frag47 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0xad413f92,
        0x1757,
        0x501d,
        {0xb2, 0x4a, 0x76, 0x5e, 0x6b, 0x4e, 0x88, 0x3c}
    }
};

static const __midl_frag46_t __midl_frag46 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag47
};

static const __midl_frag45_t __midl_frag45 =
{ 
/* **struct _NDR64_POINTER_FORMAT */
    0x20,    /* FC64_RP */
    (NDR64_UINT8) 16 /* 0x10 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag46
};

static const __midl_frag44_t __midl_frag44 =
{ 
/* get_Stream */
    { 
    /* get_Stream */      /* procedure get_Stream */
        (NDR64_UINT32) 655683 /* 0xa0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, HasReturn */
        (NDR64_UINT32) 24 /* 0x18 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 2 /* 0x2 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* __returnValue_get_Stream__ */      /* parameter __returnValue_get_Stream__ */
        &__midl_frag45,
        { 
        /* __returnValue_get_Stream__ */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [out] */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    }
};

static const __midl_frag42_t __midl_frag42 =
0x4    /* FC64_INT16 */;

static const __midl_frag41_t __midl_frag41 =
{ 
/*  */
    (NDR64_UINT32) 1 /* 0x1 */,
    { 
    /* struct _NDR64_EXPR_VAR */
        0x3,    /* FC_EXPR_VAR */
        0x6,    /* FC64_UINT32 */
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT32) 4 /* 0x4 */
    }
};

static const __midl_frag40_t __midl_frag40 =
{ 
/*  */
    { 
    /* struct _NDR64_CONF_ARRAY_HEADER_FORMAT */
        0x41,    /* FC64_CONF_ARRAY */
        (NDR64_UINT8) 1 /* 0x1 */,
        { 
        /* struct _NDR64_CONF_ARRAY_HEADER_FORMAT */
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0
        },
        (NDR64_UINT8) 0 /* 0x0 */,
        (NDR64_UINT32) 2 /* 0x2 */,
        &__midl_frag41
    },
    { 
    /* struct _NDR64_ARRAY_ELEMENT_INFO */
        (NDR64_UINT32) 2 /* 0x2 */,
        &__midl_frag42
    }
};

static const __midl_frag39_t __midl_frag39 =
{ 
/* FLAGGED_WORD_BLOB */
    { 
    /* FLAGGED_WORD_BLOB */
        0x32,    /* FC64_CONF_STRUCT */
        (NDR64_UINT8) 3 /* 0x3 */,
        { 
        /* FLAGGED_WORD_BLOB */
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0
        },
        (NDR64_UINT8) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        &__midl_frag40
    }
};

static const __midl_frag35_t __midl_frag35 =
{ 
/* get_Information */
    { 
    /* get_Information */      /* procedure get_Information */
        (NDR64_UINT32) 4849987 /* 0x4a0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, HasReturn, ClientCorrelation */
        (NDR64_UINT32) 24 /* 0x18 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 2 /* 0x2 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* __returnValue_get_Information__ */      /* parameter __returnValue_get_Information__ */
        &__midl_frag91,
        { 
        /* __returnValue_get_Information__ */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            1
        },    /* MustSize, MustFree, [out], SimpleRef, UseCache */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    }
};

static const __midl_frag33_t __midl_frag33 =
{ 
/* remove_ConnectionReceived */
    { 
    /* remove_ConnectionReceived */      /* procedure remove_ConnectionReceived */
        (NDR64_UINT32) 524611 /* 0x80143 */,    /* auto handle */ /* IsIntrepreted, [object], HasReturn */
        (NDR64_UINT32) 24 /* 0x18 */ ,  /* Stack size */
        (NDR64_UINT32) 24 /* 0x18 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 2 /* 0x2 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* token */      /* parameter token */
        &__midl_frag31,
        { 
        /* token */
            0,
            1,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustFree, [in], ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    }
};

static const __midl_frag31_t __midl_frag31 =
{ 
/* EventRegistrationToken */
    { 
    /* EventRegistrationToken */
        0x30,    /* FC64_STRUCT */
        (NDR64_UINT8) 7 /* 0x7 */,
        { 
        /* EventRegistrationToken */
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0
        },
        (NDR64_UINT8) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */
    }
};

static const __midl_frag30_t __midl_frag30 =
{ 
/* *EventRegistrationToken */
    0x20,    /* FC64_RP */
    (NDR64_UINT8) 4 /* 0x4 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag31
};

static const __midl_frag29_t __midl_frag29 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0x0d47445c,
        0xa1be,
        0x507c,
        {0x8c, 0xa9, 0xd7, 0x28, 0xa0, 0xfe, 0x89, 0xd8}
    }
};

static const __midl_frag28_t __midl_frag28 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag29
};

static const __midl_frag27_t __midl_frag27 =
{ 
/* add_ConnectionReceived */
    { 
    /* add_ConnectionReceived */      /* procedure add_ConnectionReceived */
        (NDR64_UINT32) 786755 /* 0xc0143 */,    /* auto handle */ /* IsIntrepreted, [object], ClientMustSize, HasReturn */
        (NDR64_UINT32) 32 /* 0x20 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 56 /* 0x38 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 3 /* 0x3 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* handler */      /* parameter handler */
        &__midl_frag28,
        { 
        /* handler */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in] */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* __returnValue_add_ConnectionReceived__ */      /* parameter __returnValue_add_ConnectionReceived__ */
        &__midl_frag31,
        { 
        /* __returnValue_add_ConnectionReceived__ */
            0,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            1
        },    /* MustFree, [out], SimpleRef, UseCache */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        24 /* 0x18 */,   /* Stack offset */
    }
};

static const __midl_frag25_t __midl_frag25 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0x661712ac,
        0x63e3,
        0x5243,
        {0xa9, 0x1a, 0x64, 0x09, 0xd3, 0x3f, 0xc0, 0x2b}
    }
};

static const __midl_frag24_t __midl_frag24 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag25
};

static const __midl_frag23_t __midl_frag23 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0xd0e8ff19,
        0xba8b,
        0x5263,
        {0x95, 0x51, 0x0f, 0x05, 0x2d, 0xad, 0xa5, 0x1b}
    }
};

static const __midl_frag22_t __midl_frag22 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag23
};

static const __midl_frag21_t __midl_frag21 =
{ 
/* Invoke */
    { 
    /* Invoke */      /* procedure Invoke */
        (NDR64_UINT32) 786755 /* 0xc0143 */,    /* auto handle */ /* IsIntrepreted, [object], ClientMustSize, HasReturn */
        (NDR64_UINT32) 32 /* 0x20 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 3 /* 0x3 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* sender */      /* parameter sender */
        &__midl_frag22,
        { 
        /* sender */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in] */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* e */      /* parameter e */
        &__midl_frag24,
        { 
        /* e */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in] */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        24 /* 0x18 */,   /* Stack offset */
    }
};

static const __midl_frag16_t __midl_frag16 =
{ 
/* GetResults */
    { 
    /* GetResults */      /* procedure GetResults */
        (NDR64_UINT32) 655683 /* 0xa0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, HasReturn */
        (NDR64_UINT32) 24 /* 0x18 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 2 /* 0x2 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* results */      /* parameter results */
        &__midl_frag45,
        { 
        /* results */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [out] */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    }
};

static const __midl_frag14_t __midl_frag14 =
{ 
/* struct _NDR64_CONSTANT_IID_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 1 /* 0x1 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    {
        0xa464a93a,
        0x1a22,
        0x50c6,
        {0xb3, 0xf6, 0xc4, 0x88, 0xa5, 0xf5, 0xf8, 0x93}
    }
};

static const __midl_frag13_t __midl_frag13 =
{ 
/* *struct _NDR64_POINTER_FORMAT */
    0x24,    /* FC64_IP */
    (NDR64_UINT8) 0 /* 0x0 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag14
};

static const __midl_frag12_t __midl_frag12 =
{ 
/* **struct _NDR64_POINTER_FORMAT */
    0x20,    /* FC64_RP */
    (NDR64_UINT8) 16 /* 0x10 */,
    (NDR64_UINT16) 0 /* 0x0 */,
    &__midl_frag13
};

static const __midl_frag11_t __midl_frag11 =
{ 
/* get_Completed */
    { 
    /* get_Completed */      /* procedure get_Completed */
        (NDR64_UINT32) 655683 /* 0xa0143 */,    /* auto handle */ /* IsIntrepreted, [object], ServerMustSize, HasReturn */
        (NDR64_UINT32) 24 /* 0x18 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 2 /* 0x2 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* handler */      /* parameter handler */
        &__midl_frag12,
        { 
        /* handler */
            1,
            1,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [out] */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    }
};

static const __midl_frag7_t __midl_frag7 =
{ 
/* put_Completed */
    { 
    /* put_Completed */      /* procedure put_Completed */
        (NDR64_UINT32) 786755 /* 0xc0143 */,    /* auto handle */ /* IsIntrepreted, [object], ClientMustSize, HasReturn */
        (NDR64_UINT32) 24 /* 0x18 */ ,  /* Stack size */
        (NDR64_UINT32) 0 /* 0x0 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 2 /* 0x2 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* handler */      /* parameter handler */
        &__midl_frag13,
        { 
        /* handler */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in] */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    }
};

static const __midl_frag2_t __midl_frag2 =
{ 
/* Invoke */
    { 
    /* Invoke */      /* procedure Invoke */
        (NDR64_UINT32) 786755 /* 0xc0143 */,    /* auto handle */ /* IsIntrepreted, [object], ClientMustSize, HasReturn */
        (NDR64_UINT32) 32 /* 0x20 */ ,  /* Stack size */
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT32) 8 /* 0x8 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 0 /* 0x0 */,
        (NDR64_UINT16) 3 /* 0x3 */,
        (NDR64_UINT16) 0 /* 0x0 */
    },
    { 
    /* asyncInfo */      /* parameter asyncInfo */
        &__midl_frag86,
        { 
        /* asyncInfo */
            1,
            1,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* MustSize, MustFree, [in] */
        (NDR64_UINT16) 0 /* 0x0 */,
        8 /* 0x8 */,   /* Stack offset */
    },
    { 
    /* status */      /* parameter status */
        &__midl_frag93,
        { 
        /* status */
            0,
            0,
            0,
            1,
            0,
            0,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [in], Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        16 /* 0x10 */,   /* Stack offset */
    },
    { 
    /* HRESULT */      /* parameter HRESULT */
        &__midl_frag93,
        { 
        /* HRESULT */
            0,
            0,
            0,
            0,
            1,
            1,
            1,
            1,
            0,
            0,
            0,
            0,
            0,
            (NDR64_UINT16) 0 /* 0x0 */,
            0
        },    /* [out], IsReturn, Basetype, ByValue */
        (NDR64_UINT16) 0 /* 0x0 */,
        24 /* 0x18 */,   /* Stack offset */
    }
};

static const __midl_frag1_t __midl_frag1 =
(NDR64_UINT32) 0 /* 0x0 */;
#ifdef __cplusplus
}
#endif


#include "poppack.h"


XFG_TRAMPOLINES64(HSTRING)

static const USER_MARSHAL_ROUTINE_QUADRUPLE NDR64_UserMarshalRoutines[ WIRE_MARSHAL_TABLE_SIZE ] = 
        {
            
            {
            (USER_MARSHAL_SIZING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserSize64)
            ,(USER_MARSHAL_MARSHALLING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserMarshal64)
            ,(USER_MARSHAL_UNMARSHALLING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserUnmarshal64)
            ,(USER_MARSHAL_FREEING_ROUTINE)XFG_TRAMPOLINE_FPTR(HSTRING_UserFree64)
            
            }
            

        };



/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0000, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2425, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0001, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: IUnknown, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0xC0,0x00,0x00,0x00,0x00,0x00,0x00,0x46}} */


/* Object interface: __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream, ver. 0.0,
   GUID={0xa464a93a,0x1a22,0x50c6,{0xb3,0xf6,0xc4,0x88,0xa5,0xf5,0xf8,0x93}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_Ndr64ProcTable[] =
    {
    &__midl_frag2
    };


static const MIDL_SYNTAX_INFO __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(4) ___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamProxyVtbl = 
{
    &__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_ProxyInfo,
    &IID___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    ObjectStublessClient3 /* __FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream::Invoke */
};

const CInterfaceStubVtbl ___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamStubVtbl =
{
    &IID___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream,
    &__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream_ServerInfo,
    4,
    0, /* pure interpreted */
    CStdStubBuffer_METHODS
};


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0002, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2426, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0003, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: IInspectable, ver. 0.0,
   GUID={0xAF86E2E0,0xB12D,0x4c6a,{0x9C,0x5A,0xD7,0xAA,0x65,0x10,0x1E,0x90}} */


/* Object interface: __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream, ver. 0.0,
   GUID={0xf68136b7,0xf950,0x5f98,{0x93,0x99,0x31,0x0a,0x0d,0xf7,0x3a,0xd6}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag7,
    &__midl_frag11,
    &__midl_frag16
    };


static const MIDL_SYNTAX_INFO __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(9) ___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamProxyVtbl = 
{
    &__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_ProxyInfo,
    &IID___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream::put_Completed */ ,
    ObjectStublessClient7 /* __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream::get_Completed */ ,
    ObjectStublessClient8 /* __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream::GetResults */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3,
    NdrStubCall3,
    NdrStubCall3
};

CInterfaceStubVtbl ___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamStubVtbl =
{
    &IID___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream,
    &__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_ServerInfo,
    9,
    &__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0004, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio2Eidl_0000_2427, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0005, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs, ver. 0.0,
   GUID={0x0d47445c,0xa1be,0x507c,{0x8c,0xa9,0xd7,0x28,0xa0,0xfe,0x89,0xd8}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_Ndr64ProcTable[] =
    {
    &__midl_frag21
    };


static const MIDL_SYNTAX_INFO __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(4) ___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsProxyVtbl = 
{
    &__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_ProxyInfo,
    &IID___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    ObjectStublessClient3 /* __FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs::Invoke */
};

const CInterfaceStubVtbl ___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsStubVtbl =
{
    &IID___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs,
    &__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs_ServerInfo,
    4,
    0, /* pure interpreted */
    CStdStubBuffer_METHODS
};


/* Standard interface: __MIDL_itf_Pbk201912EWinAsync2EComio_0000_0006, ver. 0.0,
   GUID={0x00000000,0x0000,0x0000,{0x00,0x00,0x00,0x00,0x00,0x00,0x00,0x00}} */


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener, ver. 0.0,
   GUID={0xD0E8FF19,0xBA8B,0x5263,{0x95,0x51,0x0F,0x05,0x2D,0xAD,0xA5,0x1B}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag27,
    &__midl_frag33,
    &__midl_frag35
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(9) ___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener::add_ConnectionReceived */ ,
    ObjectStublessClient7 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener::remove_ConnectionReceived */ ,
    ObjectStublessClient8 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener::get_Information */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3,
    NdrStubCall3,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_ServerInfo,
    9,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs, ver. 0.0,
   GUID={0x661712AC,0x63E3,0x5243,{0xA9,0x1A,0x64,0x09,0xD3,0x3F,0xC0,0x2B}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag44,
    &__midl_frag89
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(8) ___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs::get_Stream */ ,
    ObjectStublessClient7 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs::get_RemoteEndpoint */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_ServerInfo,
    8,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind, ver. 0.0,
   GUID={0x38DEC2E0,0x5F8F,0x5B4B,{0x8C,0xEB,0xE5,0x7D,0x53,0x39,0x0C,0x1C}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag75,
    &__midl_frag75
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(8) ___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind::BindEndpointAsync */ ,
    ObjectStublessClient7 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind::BindServiceNameAsync */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_ServerInfo,
    8,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector, ver. 0.0,
   GUID={0x7668644D,0x7C82,0x57DC,{0xA3,0x71,0xB7,0x40,0x4F,0x3E,0x99,0xB0}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag82
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(7) ___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector::ConnectAsync */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_ServerInfo,
    7,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind, ver. 0.0,
   GUID={0x4E5DB519,0x5FB8,0x5E37,{0x81,0xC8,0xB0,0x8A,0x4A,0x10,0xB4,0xFA}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag75
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(7) ___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind::BindAsync */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_ServerInfo,
    7,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector, ver. 0.0,
   GUID={0x9AD13FB2,0x9C7F,0x5286,{0xA9,0xCF,0x71,0xF9,0xB4,0xC3,0xE0,0x8F}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag82
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(7) ___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector::ConnectAsync */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_ServerInfo,
    7,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CITransport, ver. 0.0,
   GUID={0x9455E7DE,0x0C4F,0x5D23,{0x90,0x2A,0xA4,0x4E,0xBF,0x0A,0x4A,0xE6}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag89
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(7) ___x_ABI_CPbk20191_CWinAsync_CComio_CITransportProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransport,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CITransport::get_Transport */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CITransport_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransport,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_ServerInfo,
    7,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransport_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream, ver. 0.0,
   GUID={0xAD413F92,0x1757,0x501D,{0xB2,0x4A,0x76,0x5E,0x6B,0x4E,0x88,0x3C}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    0
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(6) ___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamProxyVtbl = 
{
    0,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_ServerInfo,
    6,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind, ver. 0.0,
   GUID={0x70FACC3F,0x8F25,0x53F5,{0x8B,0xA3,0x9C,0x91,0x9F,0x1F,0x95,0x34}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag94
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(7) ___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind::BindAsync */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_ServerInfo,
    7,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};


/* Object interface: __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector, ver. 0.0,
   GUID={0x79876E3D,0xA244,0x5AA3,{0x93,0xFE,0xFF,0x04,0x0F,0x15,0x22,0xEC}} */

#pragma code_seg(".orpc")
static const FormatInfoRef __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_Ndr64ProcTable[] =
    {
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    (FormatInfoRef)(LONG_PTR) -1,
    &__midl_frag101
    };


static const MIDL_SYNTAX_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_SyntaxInfo [  2 ] = 
    {
    {
    {{0x8A885D04,0x1CEB,0x11C9,{0x9F,0xE8,0x08,0x00,0x2B,0x10,0x48,0x60}},{2,0}},
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FormatStringOffsetTable[-3],
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    UserMarshalRoutines,
    0,
    0
    }
    ,{
    {{0x71710533,0xbeba,0x4937,{0x83,0x19,0xb5,0xdb,0xef,0x9c,0xcc,0x36}},{1,0}},
    0,
    0 ,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_Ndr64ProcTable[-3],
    0,
    NDR64_UserMarshalRoutines,
    0,
    0
    }
    };

static const MIDL_STUBLESS_PROXY_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ProxyInfo =
    {
    &Object_StubDesc,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FormatStringOffsetTable[-3],
    (RPC_SYNTAX_IDENTIFIER*)&_RpcTransferSyntax_2_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_SyntaxInfo
    
    };


static const MIDL_SERVER_INFO __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ServerInfo = 
    {
    &Object_StubDesc,
    0,
    Pbk201912EWinAsync2EComio__MIDL_ProcFormatString.Format,
    (unsigned short *) &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_FormatStringOffsetTable[-3],
    0,
    (RPC_SYNTAX_IDENTIFIER*)&_NDR64_RpcTransferSyntax_1_0,
    2,
    (MIDL_SYNTAX_INFO*)__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_SyntaxInfo
    };
const CINTERFACE_PROXY_VTABLE(7) ___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorProxyVtbl = 
{
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ProxyInfo,
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector,
    IUnknown_QueryInterface_Proxy,
    IUnknown_AddRef_Proxy,
    IUnknown_Release_Proxy ,
    NdrProxyForwardingFunction3 /* IInspectable::GetIids */ ,
    NdrProxyForwardingFunction4 /* IInspectable::GetRuntimeClassName */ ,
    NdrProxyForwardingFunction5 /* IInspectable::GetTrustLevel */ ,
    ObjectStublessClient6 /* __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector::ConnectAsync */
};


EXTERN_C DECLSPEC_SELECTANY const PRPC_STUB_FUNCTION __x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_table[] =
{
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    STUB_FORWARDING_FUNCTION,
    NdrStubCall3
};

CInterfaceStubVtbl ___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorStubVtbl =
{
    &IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_ServerInfo,
    7,
    &__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector_table[-3],
    CStdStubBuffer_DELEGATING_METHODS
};

#ifdef __cplusplus
namespace {
#endif
static const MIDL_STUB_DESC Object_StubDesc = 
    {
    0,
    NdrOleAllocate,
    NdrOleFree,
    0,
    0,
    0,
    0,
    0,
    Pbk201912EWinAsync2EComio__MIDL_TypeFormatString.Format,
    1, /* -error bounds_check flag */
    0x60003, /* Ndr library version */
    0,
    0x8010274, /* MIDL Version 8.1.628 */
    0,
    UserMarshalRoutines,
    0,  /* notify & notify_flag routine table */
    0x2000001, /* MIDL flag */
    0, /* cs routines */
    0,   /* proxy/server info */
    0
    };
#ifdef __cplusplus
}
#endif

const CInterfaceProxyVtbl * const _Pbk201912EWinAsync2EComio_ProxyVtblList[] = 
{
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerProxyVtbl,
    ( CInterfaceProxyVtbl *) &___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorProxyVtbl,
    ( CInterfaceProxyVtbl *) &___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorProxyVtbl,
    ( CInterfaceProxyVtbl *) &___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CITransportProxyVtbl,
    ( CInterfaceProxyVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindProxyVtbl,
    0
};

const CInterfaceStubVtbl * const _Pbk201912EWinAsync2EComio_StubVtblList[] = 
{
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBindStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListenerStubVtbl,
    ( CInterfaceStubVtbl *) &___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStreamStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnectorStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBindStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnectorStubVtbl,
    ( CInterfaceStubVtbl *) &___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgsStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStreamStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgsStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnectorStubVtbl,
    ( CInterfaceStubVtbl *) &___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStreamStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStubVtbl,
    ( CInterfaceStubVtbl *) &___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBindStubVtbl,
    0
};

PCInterfaceName const _Pbk201912EWinAsync2EComio_InterfaceNamesList[] = 
{
    "__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener",
    "__FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector",
    "__FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector",
    "__FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CITransport",
    "__x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind",
    0
};

const IID *  const _Pbk201912EWinAsync2EComio_BaseIIDList[] = 
{
    &IID_IInspectable,
    &IID_IInspectable,
    0,
    &IID_IInspectable,
    &IID_IInspectable,
    &IID_IInspectable,
    0,
    &IID_IInspectable,
    &IID_IInspectable,
    &IID_IInspectable,
    &IID_IInspectable,
    &IID_IInspectable,
    &IID_IInspectable,
    0
};


#define _Pbk201912EWinAsync2EComio_CHECK_IID(n)	IID_GENERIC_CHECK_IID( _Pbk201912EWinAsync2EComio, pIID, n)

int __stdcall _Pbk201912EWinAsync2EComio_IID_Lookup( const IID * pIID, int * pIndex )
{
    IID_BS_LOOKUP_SETUP

    IID_BS_LOOKUP_INITIAL_TEST( _Pbk201912EWinAsync2EComio, 13, 8 )
    IID_BS_LOOKUP_NEXT_TEST( _Pbk201912EWinAsync2EComio, 4 )
    IID_BS_LOOKUP_NEXT_TEST( _Pbk201912EWinAsync2EComio, 2 )
    IID_BS_LOOKUP_NEXT_TEST( _Pbk201912EWinAsync2EComio, 1 )
    IID_BS_LOOKUP_RETURN_RESULT( _Pbk201912EWinAsync2EComio, 13, *pIndex )
    
}

EXTERN_C const ExtendedProxyFileInfo Pbk201912EWinAsync2EComio_ProxyFileInfo = 
{
    (PCInterfaceProxyVtblList *) & _Pbk201912EWinAsync2EComio_ProxyVtblList,
    (PCInterfaceStubVtblList *) & _Pbk201912EWinAsync2EComio_StubVtblList,
    (const PCInterfaceName * ) & _Pbk201912EWinAsync2EComio_InterfaceNamesList,
    (const IID ** ) & _Pbk201912EWinAsync2EComio_BaseIIDList,
    & _Pbk201912EWinAsync2EComio_IID_Lookup, 
    13,
    2,
    0, /* table of [async_uuid] interfaces */
    0, /* Filler1 */
    0, /* Filler2 */
    0  /* Filler3 */
};
#if _MSC_VER >= 1200
#pragma warning(pop)
#endif


#endif /* defined(_M_AMD64)*/

