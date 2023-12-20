use frame_support::{
    parameter_types,
    traits::{Get, ProcessMessageError},
};
use parity_scale_codec::{Compact, Decode, Encode};
use sp_core::{crypto::AccountId32, ConstU32};
use staging_xcm::{
    latest::{prelude::*, InteriorMultiLocation, MultiLocation, NetworkId},
    DoubleEncoded,
};
use staging_xcm_builder::{CreateMatcher, DescribeAllTerminal, DescribeLocation, MatchXcm};
use staging_xcm_executor::traits::{Properties, ShouldExecute};
use std::{cell::Cell, marker::PhantomData, ops::ControlFlow};

pub struct NewWithComputedOrigin<InnerBarrier, LocalUniversal, MaxPrefixes>(
    PhantomData<(InnerBarrier, LocalUniversal, MaxPrefixes)>,
);

impl<
        InnerBarrier: ShouldExecute,
        LocalUniversal: Get<InteriorMultiLocation>,
        MaxPrefixes: Get<u32>,
    > ShouldExecute for NewWithComputedOrigin<InnerBarrier, LocalUniversal, MaxPrefixes>
{
    fn should_execute<Call>(
        origin: &MultiLocation,
        instructions: &mut [Instruction<Call>],
        max_weight: Weight,
        properties: &mut Properties,
    ) -> Result<(), ProcessMessageError> {
        let mut actual_origin = *origin;
        let skipped = Cell::new(0usize);
        instructions.matcher().match_next_inst_while(
            |_| skipped.get() < MaxPrefixes::get() as usize,
            |inst| {
                match inst {
                    UniversalOrigin(new_global) => {
                        // ↓↓ ORIGINAL CODE ↓↓

                        // Note the origin is *relative to local consensus*! So we need to escape
                        // local consensus with the `parents` before diving in into the
                        // `universal_location`.
                        // actual_origin = X1(*new_global).relative_to(&LocalUniversal::get());

                        // ↑↑ ORIGINAL CODE ↑↑

                        // ↓↓ NEW CODE ↓↓

                        actual_origin = X1(GlobalConsensus(
                            LocalUniversal::get()
                                .global_consensus()
                                .map_err(|_| ProcessMessageError::Unsupported)?,
                        ))
                        .within_global(
                            actual_origin
                                .prepended_with(LocalUniversal::get().relative_to(&X1(*new_global)))
                                .map_err(|_| ProcessMessageError::Unsupported)?,
                        )
                        .map_err(|_| ProcessMessageError::Unsupported)?
                        .into_location();

                        // ↑↑ NEW CODE ↑↑
                    }
                    DescendOrigin(j) => {
                        let Ok(_) = actual_origin.append_with(*j) else {
                            return Err(ProcessMessageError::Unsupported);
                        };
                    }
                    _ => return Ok(ControlFlow::Break(())),
                };
                skipped.set(skipped.get() + 1);
                Ok(ControlFlow::Continue(()))
            },
        )?;
        InnerBarrier::should_execute(
            &actual_origin,
            &mut instructions[skipped.get()..],
            max_weight,
            properties,
        )
    }
}

pub struct NewDescribeFamily<DescribeInterior>(PhantomData<DescribeInterior>);
impl<Suffix: DescribeLocation> DescribeLocation for NewDescribeFamily<Suffix> {
    fn describe_location(l: &MultiLocation) -> Option<Vec<u8>> {
        match (l.parents, l.interior.first()) {
            (0, Some(Parachain(index))) => {
                let tail = l.interior.split_first().0;
                let interior = Suffix::describe_location(&tail.into())?;
                Some((b"ChildChain", Compact::<u32>::from(*index), interior).encode())
            }
            (1, Some(Parachain(index))) => {
                let tail = l.interior.split_first().0;
                let interior = Suffix::describe_location(&tail.into())?;
                Some((b"SiblingChain", Compact::<u32>::from(*index), interior).encode())
            }
            (1, _) => {
                let tail = l.interior.into();
                let interior = Suffix::describe_location(&tail)?;
                Some((b"ParentChain", interior).encode())
            }

            // ↓↓ NEW CODE ↓↓
            (0, Some(GlobalConsensus(network_id))) => {
                let tail = l.interior.split_first().0;
                match tail.first() {
                    Some(Parachain(index)) => {
                        let tail = tail.split_first().0;
                        let interior = Suffix::describe_location(&tail.into())?;
                        Some(
                            (
                                b"UniversalLocation",
                                *network_id,
                                b"Parachain",
                                Compact::<u32>::from(*index),
                                interior,
                            )
                                .encode(),
                        )
                    }
                    _ => return None,
                }
            }
            // ↑↑ NEW CODE ↑↑
            _ => return None,
        }
    }
}

parameter_types! {
    pub RelayUniversalLocation: InteriorMultiLocation = X1(GlobalConsensus(NetworkId::Kusama));
    pub ParaUniversalLocation: InteriorMultiLocation = X2(GlobalConsensus(NetworkId::Kusama), Parachain(1000));
}

pub type RelayBarrier =
    NewWithComputedOrigin<DeriveAccountBarrier, RelayUniversalLocation, ConstU32<8>>;

pub type ParaBarrier =
    NewWithComputedOrigin<DeriveAccountBarrier, ParaUniversalLocation, ConstU32<8>>;

pub struct DeriveAccountBarrier;
impl ShouldExecute for DeriveAccountBarrier {
    fn should_execute<Call>(
        origin: &MultiLocation,
        _instructions: &mut [Instruction<Call>],
        _max_weight: Weight,
        _properties: &mut Properties,
    ) -> Result<(), ProcessMessageError> {
        eprintln!("origin: {:?}", origin);

        let account = AccountId32::decode(
            &mut NewDescribeFamily::<DescribeAllTerminal>::describe_location(origin)
                .unwrap()
                .as_slice(),
        );

        eprintln!("account: {:?}", account);

        Ok(())
    }
}

fn main() {
    let origin_from_relay_perspective = MultiLocation {
        parents: 0,
        interior: Junctions::X1(Junction::Parachain(2125)),
    };

    let origin_from_para_perspective = MultiLocation {
        parents: 1,
        interior: Junctions::X1(Junction::Parachain(2125)),
    };

    let mut instructions: Vec<Instruction<()>> = vec![
        Instruction::UniversalOrigin(Junction::GlobalConsensus(NetworkId::Kusama)),
        Instruction::DescendOrigin(Junctions::X1(Junction::Plurality {
            id: BodyId::Index(0),
            part: BodyPart::Voice,
        })),
        Instruction::Transact {
            origin_kind: OriginKind::Native,
            require_weight_at_most: Weight::from_parts(0, 0),
            call: <DoubleEncoded<()> as From<Vec<u8>>>::from(Vec::<u8>::new()),
        },
    ];

    eprintln!("From the relay:");
    <RelayBarrier as ShouldExecute>::should_execute(
        &origin_from_relay_perspective,
        &mut instructions,
        Weight::from_parts(100, 100),
        &mut Properties {
            weight_credit: Weight::from_parts(100, 100),
            message_id: None,
        },
    )
    .unwrap();

    eprintln!();
    eprintln!("From the para:");

    <ParaBarrier as ShouldExecute>::should_execute(
        &origin_from_para_perspective,
        &mut instructions,
        Weight::from_parts(100, 100),
        &mut Properties {
            weight_credit: Weight::from_parts(100, 100),
            message_id: None,
        },
    )
    .unwrap();
}