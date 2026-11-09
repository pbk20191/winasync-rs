

/* this ALWAYS GENERATED file contains the IIDs and CLSIDs */

/* link this file in with the server and any clients */


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



#ifdef __cplusplus
extern "C"{
#endif 


#include <rpc.h>
#include <rpcndr.h>

#ifdef _MIDL_USE_GUIDDEF_

#ifndef INITGUID
#define INITGUID
#include <guiddef.h>
#undef INITGUID
#else
#include <guiddef.h>
#endif

#define MIDL_DEFINE_GUID(type,name,l,w1,w2,b1,b2,b3,b4,b5,b6,b7,b8) \
        DEFINE_GUID(name,l,w1,w2,b1,b2,b3,b4,b5,b6,b7,b8)

#else // !_MIDL_USE_GUIDDEF_

#ifndef __IID_DEFINED__
#define __IID_DEFINED__

typedef struct _IID
{
    unsigned long x;
    unsigned short s1;
    unsigned short s2;
    unsigned char  c[8];
} IID;

#endif // __IID_DEFINED__

#ifndef CLSID_DEFINED
#define CLSID_DEFINED
typedef IID CLSID;
#endif // CLSID_DEFINED

#define MIDL_DEFINE_GUID(type,name,l,w1,w2,b1,b2,b3,b4,b5,b6,b7,b8) \
        EXTERN_C __declspec(selectany) const type name = {l,w1,w2,{b1,b2,b3,b4,b5,b6,b7,b8}}

#endif // !_MIDL_USE_GUIDDEF_

MIDL_DEFINE_GUID(IID, IID___FIAsyncOperationCompletedHandler_1_Pbk20191__CWinAsync__CComio__CITransportStream,0xa464a93a,0x1a22,0x50c6,0xb3,0xf6,0xc4,0x88,0xa5,0xf5,0xf8,0x93);


MIDL_DEFINE_GUID(IID, IID___FIAsyncOperation_1_Pbk20191__CWinAsync__CComio__CITransportStream,0xf68136b7,0xf950,0x5f98,0x93,0x99,0x31,0x0a,0x0d,0xf7,0x3a,0xd6);


MIDL_DEFINE_GUID(IID, IID___FITypedEventHandler_2_Pbk20191__CWinAsync__CComio__CIConnectionListener_Pbk20191__CWinAsync__CComio__CIConnectionReceivedArgs,0x0d47445c,0xa1be,0x507c,0x8c,0xa9,0xd7,0x28,0xa0,0xfe,0x89,0xd8);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionListener,0xD0E8FF19,0xBA8B,0x5263,0x95,0x51,0x0F,0x05,0x2D,0xAD,0xA5,0x1B);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CIConnectionReceivedArgs,0x661712AC,0x63E3,0x5243,0xA9,0x1A,0x64,0x09,0xD3,0x3F,0xC0,0x2B);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVBind,0x38DEC2E0,0x5F8F,0x5B4B,0x8C,0xEB,0xE5,0x7D,0x53,0x39,0x0C,0x1C);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CIHyperVConnector,0x7668644D,0x7C82,0x57DC,0xA3,0x71,0xB7,0x40,0x4F,0x3E,0x99,0xB0);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeBind,0x4E5DB519,0x5FB8,0x5E37,0x81,0xC8,0xB0,0x8A,0x4A,0x10,0xB4,0xFA);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CINamedPipeConnector,0x9AD13FB2,0x9C7F,0x5286,0xA9,0xCF,0x71,0xF9,0xB4,0xC3,0xE0,0x8F);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransport,0x9455E7DE,0x0C4F,0x5D23,0x90,0x2A,0xA4,0x4E,0xBF,0x0A,0x4A,0xE6);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CITransportStream,0xAD413F92,0x1757,0x501D,0xB2,0x4A,0x76,0x5E,0x6B,0x4E,0x88,0x3C);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixBind,0x70FACC3F,0x8F25,0x53F5,0x8B,0xA3,0x9C,0x91,0x9F,0x1F,0x95,0x34);


MIDL_DEFINE_GUID(IID, IID___x_ABI_CPbk20191_CWinAsync_CComio_CIUnixConnector,0x79876E3D,0xA244,0x5AA3,0x93,0xFE,0xFF,0x04,0x0F,0x15,0x22,0xEC);

#undef MIDL_DEFINE_GUID

#ifdef __cplusplus
}
#endif



